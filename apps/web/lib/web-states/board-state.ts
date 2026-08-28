import {
	type FlowIrCommitDisposition,
	type FlowIrCommitDispositionResult,
	type FlowIrCommitToken,
	type IApplyFlowIrCommitResponse,
	type IApplyFlowScriptResponse,
	type IBoard,
	type IBoardMutationOptions,
	type IBoardState,
	type IBoardSummary,
	type IBoardSummaryInclude,
	type IBoardVariables,
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
	type IScopedFlowScriptResponse,
	type IVersionType,
	type ProgressToastData,
	checkOAuthTokens,
	finishAllProgressToasts,
	showProgressToast,
} from "@flow-like/flow-like-ui";
import type {
	CanvasSettings,
	SurfaceComponent,
} from "@flow-like/flow-like-ui/components/a2ui/types";
import { apiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import {
	BoardSyncClient,
	type IBoardSyncRequest,
	type IBoardSyncResponse,
} from "@flow-like/flow-like-ui/lib/board-sync";
import type { FlowScriptApplyOrigin } from "@flow-like/flow-like-ui/lib/flowscript-apply-failure";
import type {
	ChatImage,
	CopilotScope,
	CopilotToolContext,
	UIActionContext,
	UnifiedChatMessage,
	UnifiedCopilotResponse,
} from "@flow-like/flow-like-ui/lib/schema/copilot";
import { normalizeBoardVersion } from "@flow-like/flow-like-ui/lib/schema/flow/board-version";
import type { IElementDemand } from "@flow-like/flow-like-ui/lib/schema/flow/element-demand";
import type { IPrerunBoardResponse } from "@flow-like/flow-like-ui/state/backend-state/types";
import { globalChatTransportRunId } from "@flow-like/flow-like-ui/state/global-chat/global-chat-run-control";
import { runGlobalChatTool } from "@flow-like/flow-like-ui/state/global-chat/global-chat-tool-registry";
import { dispatchSpecialistToolRequest } from "@flow-like/flow-like-ui/state/global-chat/global-chat-web-transport";
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
import { executionElementsFromResponse } from "./execution-elements";

/** The request id inside a `tool_request` SSE payload, or undefined for a malformed frame. */
function toolRequestIdOf(data: string): string | undefined {
	try {
		const parsed = JSON.parse(data) as { requestId?: unknown };
		return typeof parsed.requestId === "string" && parsed.requestId.trim()
			? parsed.requestId
			: undefined;
	} catch {
		return undefined;
	}
}

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

/**
 * `POST /board/{id}` answers with a bare command list for requests without `sync`, and with
 * `{commands, sync}` when the request carried the held manifest. Both are accepted so a client
 * talking to a hub of either generation keeps working.
 */
type ExecuteCommandsWire =
	| IGenericCommand[]
	| { commands: IGenericCommand[]; sync?: IBoardSyncResponse | null };

function normalizeExecuteCommandsWire(wire: ExecuteCommandsWire): {
	commands: IGenericCommand[];
	sync: IBoardSyncResponse | undefined;
} {
	if (Array.isArray(wire)) return { commands: wire, sync: undefined };
	return { commands: wire.commands ?? [], sync: wire.sync ?? undefined };
}

export class WebBoardState implements IBoardState {
	private readonly copilotAbortControllers = new Map<string, AbortController>();
	private readonly appIdByBoardId = new Map<string, string>();
	private readonly boardSync = new BoardSyncClient();

	constructor(private readonly backend: WebBackendRef) {}

	async getBoards(appId: string): Promise<IBoard[]> {
		try {
			const boards = await apiGet<IBoard[]>(
				`apps/${appId}/board`,
				this.backend.auth,
			);
			for (const board of boards) {
				this.appIdByBoardId.set(board.id, appId);
			}
			return boards;
		} catch {
			return [];
		}
	}

	async getBoardSummaries(
		appId: string,
		include?: IBoardSummaryInclude[],
	): Promise<IBoardSummary[]> {
		const query = include?.length ? `?include=${include.join(",")}` : "";
		const summaries = await apiGet<IBoardSummary[]>(
			`apps/${appId}/board/summaries${query}`,
			this.backend.auth,
		);
		for (const summary of summaries) {
			this.appIdByBoardId.set(summary.id, appId);
		}
		return summaries;
	}

	async getBoardVariables(appId: string): Promise<IBoardVariables[]> {
		return apiGet<IBoardVariables[]>(
			`apps/${appId}/board/variables`,
			this.backend.auth,
		);
	}

	async getCatalog(appId: string): Promise<INode[]> {
		try {
			const nodes = await apiGet<INode[]>(
				`apps/${appId}/nodes`,
				this.backend.auth,
			);
			// Lets subsequent board syncs ship catalog-owned node fields lean.
			this.boardSync.setCatalog(appId, nodes);
			return nodes;
		} catch {
			return [];
		}
	}

	async getBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		_forceFresh?: boolean,
	): Promise<IBoard> {
		const params = version ? `?version=${version.join("_")}` : "";
		const board = await this.boardSync.sync(
			appId,
			boardId,
			version,
			(request: IBoardSyncRequest) =>
				apiPost<IBoardSyncResponse>(
					`apps/${appId}/board/${boardId}/sync${params}`,
					request,
					this.backend.auth,
				),
		);
		this.appIdByBoardId.set(boardId, appId);

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
		const board = await this.getBoard(
			appId,
			boardId,
			normalizeBoardVersion(payload.version),
		);
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

		const headers: Record<string, string> = {
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
					version: payload.version,
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
		includeNodes?: boolean,
	): Promise<ILogMetadata[]> {
		const params = new URLSearchParams();
		if (nodeId) params.set("node_id", nodeId);
		if (from !== undefined) params.set("from", from.toString());
		if (to !== undefined) params.set("to", to.toString());
		if (status) params.set("status", status);
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());
		if (includeNodes) params.set("include_nodes", "true");

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
		const appId = this.appIdByBoardId.get(boardId);
		if (appId) this.boardSync.forget(appId, boardId);
	}

	async executeCommand(
		appId: string,
		boardId: string,
		command: IGenericCommand,
		options?: IBoardMutationOptions,
	): Promise<IGenericCommand> {
		const results = await this.executeCommands(
			appId,
			boardId,
			[command],
			options,
		);
		return results[0];
	}

	/**
	 * One round trip: the batch travels with the held sync manifest and the hub answers with the
	 * commands plus the diff against the revision it produced. The diff is applied onto the held
	 * board only if it is still the revision the manifest described (see
	 * `BoardSyncClient.ingest`); otherwise the caller refetches as before.
	 */
	async executeCommands(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
		options?: IBoardMutationOptions,
	): Promise<IGenericCommand[]> {
		const sync = this.boardSync.syncRequest(appId, boardId, undefined);
		const wire = await apiPost<ExecuteCommandsWire>(
			`apps/${appId}/board/${boardId}`,
			sync ? { commands, sync } : { commands },
			this.backend.auth,
		);
		const { commands: results, sync: tail } =
			normalizeExecuteCommandsWire(wire);
		if (sync && tail && options?.onBoard) {
			const board = this.boardSync.ingest(
				appId,
				boardId,
				undefined,
				sync,
				tail,
			);
			if (board) {
				await this.presignMediaComments(appId, boardId, board);
				options.onBoard(board);
			}
		}
		return results;
	}

	async applyFlowScript(
		appId: string,
		boardId: string,
		flowscript: string,
		currentLayer?: string,
		catalogNodes?: INode[],
		allowDeletions = false,
		origin: FlowScriptApplyOrigin = "editor",
		scopeAnchors?: string[],
		module?: string,
	): Promise<IApplyFlowScriptResponse> {
		return apiPost<IApplyFlowScriptResponse>(
			`apps/${appId}/board/${boardId}/flowscript/apply`,
			{
				flowscript,
				current_layer: currentLayer,
				allow_deletions: allowDeletions,
				// The endpoint captures failed applies; without this every FlowPilot attempt would
				// be indistinguishable from a person's edit in the admin view.
				origin,
				scope_anchors: scopeAnchors,
				module,
			},
			this.backend.auth,
		);
	}

	async flowIrCommitDisposition(
		token: FlowIrCommitToken,
		disposition: FlowIrCommitDisposition,
	): Promise<FlowIrCommitDispositionResult> {
		const appId = this.appIdByBoardId.get(token.board_id);
		if (!appId) {
			throw new Error(
				"The app owning this compiled workflow review is unavailable",
			);
		}
		return apiPost<FlowIrCommitDispositionResult>(
			`apps/${appId}/board/${token.board_id}/flow-ir-commit/disposition`,
			{ token, disposition },
			this.backend.auth,
		);
	}

	async applyFlowIrCommit(
		appId: string,
		token: FlowIrCommitToken,
	): Promise<IApplyFlowIrCommitResponse> {
		this.appIdByBoardId.set(token.board_id, appId);
		if (token.requires_destructive_approval === true) {
			const approved =
				typeof window !== "undefined" &&
				window.confirm(
					"This FlowScript review removes or replaces existing workflow items. Delete them and apply the exact compiled review?",
				);
			if (!approved) {
				return {
					status: "error",
					code: "IR_COMMIT_DESTRUCTIVE_APPROVAL_DENIED",
					message:
						"Destructive workflow approval was cancelled. Nothing was applied and the review remains pending.",
					commands: [],
					board_commands: [],
					diagnostics: [],
				};
			}
		}
		return apiPost<IApplyFlowIrCommitResponse>(
			`apps/${appId}/board/${token.board_id}/flow-ir-commit/apply`,
			{
				token,
				approve_destructive: token.requires_destructive_approval === true,
			},
			this.backend.auth,
		);
	}

	async getFlowScript(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		anchors = true,
	): Promise<string> {
		const params = new URLSearchParams();
		if (version) params.set("version", version.join("_"));
		params.set("anchors", String(anchors));
		const response = await apiGet<{ flowscript: string }>(
			`apps/${appId}/board/${boardId}/flowscript?${params}`,
			this.backend.auth,
		);
		return response.flowscript;
	}

	async getFlowScriptScoped(
		appId: string,
		boardId: string,
		nodeIds: string[],
		anchors = true,
	): Promise<IScopedFlowScriptResponse> {
		const params = new URLSearchParams();
		params.set("anchors", String(anchors));
		params.set("node_ids", nodeIds.join(","));
		const response = await apiGet<{
			flowscript: string;
			scope_anchors?: string[];
		}>(
			`apps/${appId}/board/${boardId}/flowscript?${params}`,
			this.backend.auth,
		);
		return {
			flowscript: response.flowscript,
			scope_anchors: response.scope_anchors ?? [],
		};
	}

	async getFlowScriptFile(
		appId: string,
		boardId: string,
		file: string,
		anchors = true,
	): Promise<IScopedFlowScriptResponse> {
		const params = new URLSearchParams();
		params.set("anchors", String(anchors));
		params.set("file", file);
		const response = await apiGet<{
			flowscript: string;
			scope_anchors?: string[];
		}>(
			`apps/${appId}/board/${boardId}/flowscript?${params}`,
			this.backend.auth,
		);
		return {
			flowscript: response.flowscript,
			scope_anchors: response.scope_anchors ?? [],
		};
	}

	async formatFlowScript(
		appId: string,
		boardId: string,
		flowscript: string,
		anchors = true,
	): Promise<string> {
		const response = await apiPost<{ flowscript: string }>(
			`apps/${appId}/board/${boardId}/flowscript/format`,
			{ flowscript, anchors },
			this.backend.auth,
		);
		return response.flowscript;
	}

	async getExecutionElements(
		appId: string,
		boardId: string,
		pageId: string,
		wildcard?: boolean,
		version?: [number, number, number],
	): Promise<Record<string, unknown>> {
		const params = new URLSearchParams();
		params.set("page_id", pageId);
		if (wildcard !== undefined) params.set("wildcard", String(wildcard));
		if (version) params.set("version", version.join("_"));

		try {
			const response = await apiGet<{
				elements: Record<string, unknown>;
			}>(
				`apps/${appId}/board/${boardId}/elements?${params}`,
				this.backend.auth,
			);
			return executionElementsFromResponse(response);
		} catch {
			return {};
		}
	}

	async getElementDemand(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IElementDemand> {
		const query = version ? `?version=${version.join("_")}` : "";
		return await apiGet<IElementDemand>(
			`apps/${appId}/board/${boardId}/element-demand${query}`,
			this.backend.auth,
		);
	}

	async copilot_chat(
		scope: CopilotScope,
		board: IBoard | null,
		catalogNodes: INode[] | undefined,
		selectedNodeIds: string[],
		currentSurface: SurfaceComponent[] | null,
		currentCanvasSettings: CanvasSettings | null,
		selectedComponentIds: string[],
		userPrompt: string,
		history: UnifiedChatMessage[],
		requestImages?: ChatImage[],
		onToken?: (token: string) => void,
		modelId?: string,
		reasoningEffort?: string,
		token?: string,
		runContext?: IRunContext,
		actionContext?: UIActionContext,
		_nested?: boolean,
		_readOnly?: boolean,
		toolContext?: CopilotToolContext,
		requestId?: string,
		rawUserPrompt?: string,
		appId?: string,
	): Promise<UnifiedCopilotResponse> {
		const contextAppId =
			appId ??
			toolContext?.appId ??
			actionContext?.app_id ??
			runContext?.app_id;
		const contextBoardId =
			toolContext?.boardId ?? board?.id ?? runContext?.board_id;
		if (contextAppId && contextBoardId) {
			this.appIdByBoardId.set(contextBoardId, contextAppId);
		}
		const baseUrl = getApiBaseUrl();
		const url = `${baseUrl}/api/v1/ai/copilot/chat`;
		// The server minted the owning chat run's id; callers only hold the client id, so a nested
		// specialist has to be addressed through the transport's mapping.
		const specialistRunId = toolContext?.runId
			? globalChatTransportRunId(toolContext.runId)
			: undefined;

		const headers: Record<string, string> = {
			"Content-Type": "application/json",
		};
		if (onToken) {
			headers.Accept = "text/event-stream";
		}
		if (this.backend.auth?.user?.access_token) {
			headers["Authorization"] =
				`Bearer ${this.backend.auth.user.access_token}`;
		}

		const wantsStream = Boolean(onToken);
		const abortController = new AbortController();
		if (requestId) {
			this.copilotAbortControllers.get(requestId)?.abort();
			this.copilotAbortControllers.set(requestId, abortController);
		}
		try {
			const response = await fetch(url, {
				method: "POST",
				headers,
				signal: abortController.signal,
				body: JSON.stringify({
					scope,
					board,
					selected_node_ids: selectedNodeIds,
					current_surface: currentSurface,
					current_canvas_settings: currentCanvasSettings,
					selected_component_ids: selectedComponentIds,
					user_prompt: userPrompt,
					raw_user_prompt: rawUserPrompt,
					conversation_id: toolContext?.conversationId,
					source_user_prompt: toolContext?.sourceUserPrompt,
					history,
					request_images: requestImages,
					model_id: modelId,
					reasoning_effort: reasoningEffort,
					token,
					run_context: runContext,
					action_context: actionContext,
					app_id: contextAppId,
					// Data Studio / Scout specialists run server-side but call their tools in this
					// browser: they ride the owning chat run so results reach the endpoint the
					// global chat already posts to, and stopping that run stops them too.
					run_id: specialistRunId,
					overlay_id: toolContext?.overlayId,
					stream: wantsStream,
				}),
			});

			if (!response.ok) {
				const errorText = await response.text();
				throw apiResponseError(response, errorText, "ai/copilot/chat");
			}

			if (
				onToken &&
				response.body &&
				response.headers.get("content-type")?.includes("text/event-stream")
			) {
				const reader = response.body.getReader();
				const decoder = new TextDecoder();
				let buffer = "";
				let result: UnifiedCopilotResponse | undefined;
				let streamError: Error | undefined;
				// A nested Data Studio / Scout specialist runs server-side but owns no tools of its
				// own: they execute here, exactly like the orchestrator's, and their results go back
				// on the channel each request carries.
				const toolDispatches = new Set<Promise<void>>();
				const seenToolRequests = new Set<string>();

				const handleEvent = (rawEvent: string) => {
					if (!rawEvent.trim()) return;

					let eventName = "message";
					const dataLines: string[] = [];

					for (const line of rawEvent.split(/\r?\n/)) {
						if (line.startsWith(":")) continue;
						if (line.startsWith("event:")) {
							eventName = line.slice("event:".length).trim();
						} else if (line.startsWith("data:")) {
							dataLines.push(line.slice("data:".length).replace(/^\s/, ""));
						}
					}

					const data = dataLines.join("\n");
					if (!data) return;

					if (eventName === "token" || eventName === "message") {
						onToken(data);
						return;
					}

					if (eventName === "tool_request") {
						// A redelivered frame must not run a mutating tool twice.
						const requestId = toolRequestIdOf(data);
						if (requestId) {
							if (seenToolRequests.has(requestId)) return;
							seenToolRequests.add(requestId);
						}
						const dispatch = dispatchSpecialistToolRequest({
							data,
							onToolRequest: runGlobalChatTool,
						}).catch((error) => {
							// The specialist is blocked on this request; failing the stream beats
							// leaving it to time out with no explanation.
							streamError ??=
								error instanceof Error ? error : new Error(String(error));
						});
						toolDispatches.add(dispatch);
						void dispatch.finally(() => toolDispatches.delete(dispatch));
						return;
					}

					if (eventName === "final") {
						try {
							result = JSON.parse(data) as UnifiedCopilotResponse;
						} catch {
							streamError = new Error("Failed to parse final Copilot response");
						}
						return;
					}

					if (eventName === "error") {
						let message = "Copilot chat failed";
						try {
							const parsed = JSON.parse(data) as { error?: string };
							message = parsed.error ?? message;
						} catch {
							message = data;
						}
						streamError = new Error(message);
					}
				};

				const consumeBufferedEvents = () => {
					let separatorIndex = buffer.search(/\r?\n\r?\n/);
					while (separatorIndex !== -1) {
						const rawEvent = buffer.slice(0, separatorIndex);
						const separatorLength =
							buffer.slice(separatorIndex, separatorIndex + 4) === "\r\n\r\n"
								? 4
								: 2;
						buffer = buffer.slice(separatorIndex + separatorLength);
						handleEvent(rawEvent);
						if (streamError) throw streamError;
						separatorIndex = buffer.search(/\r?\n\r?\n/);
					}
				};

				while (true) {
					const { done, value } = await reader.read();
					if (done) break;

					buffer += decoder.decode(value, { stream: true });
					consumeBufferedEvents();
				}

				buffer += decoder.decode();
				if (buffer.trim()) handleEvent(buffer);
				// A delivery failure for a tool still in flight when the stream ended belongs to
				// this run, so wait for the dispatches before deciding the run succeeded.
				if (toolDispatches.size > 0) {
					await Promise.allSettled([...toolDispatches]);
				}
				if (streamError) throw streamError;
				if (!result)
					throw new Error("Copilot stream ended without a final event");

				return result;
			}

			return response.json();
		} finally {
			if (
				requestId &&
				this.copilotAbortControllers.get(requestId) === abortController
			) {
				this.copilotAbortControllers.delete(requestId);
			}
		}
	}

	async cancelCopilotChat(requestId: string): Promise<void> {
		const controller = this.copilotAbortControllers.get(requestId);
		controller?.abort();
		if (this.copilotAbortControllers.get(requestId) === controller) {
			this.copilotAbortControllers.delete(requestId);
		}
	}

	async prerunBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IPrerunBoardResponse> {
		const params = version ? `?version=${version.join("_")}` : "";
		return apiGet<IPrerunBoardResponse>(
			`apps/${appId}/board/${boardId}/prerun${params}`,
			this.backend.auth,
		);
	}
}
