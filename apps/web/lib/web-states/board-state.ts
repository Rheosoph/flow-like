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
	type ILogQuery,
	type INode,
	type IOAuthProvider,
	type IRealtimeAccess,
	type IRunContext,
	type IRunLogSummary,
	type IRunPayload,
	type IScopedFlowScriptResponse,
	type IVersionType,
	checkOAuthTokens,
} from "@flow-like/flow-like-ui";
import type {
	CanvasSettings,
	SurfaceComponent,
} from "@flow-like/flow-like-ui/components/a2ui/types";
import {
	apiResponseError,
	isMissingResourceError,
	isMissingRouteError,
} from "@flow-like/flow-like-ui/lib/api-error";
import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import {
	BOARD_FORMAT_HEADER,
	type BoardFormatCapabilities,
	CURRENT_BOARD_FORMAT_VERSION,
} from "@flow-like/flow-like-ui/lib/board-format";
import {
	BoardMediaProjector,
	BoardSyncClient,
	type IBoardSyncRequest,
	type IBoardSyncResponse,
} from "@flow-like/flow-like-ui/lib/board-sync";
import {
	cancelChannel,
	isChannelHandle,
} from "@flow-like/flow-like-ui/lib/channel";
import { withDeviceCommandBridge } from "@flow-like/flow-like-ui/lib/device-bridge";
import {
	type FlowVersionState,
	parseFlowVersionState,
} from "@flow-like/flow-like-ui/lib/device-management/hub/endpoints";
import { getExecutionRuntime } from "@flow-like/flow-like-ui/lib/execution/runtime-client";
import {
	type ExecutionTask,
	getExecutionMetadata,
} from "@flow-like/flow-like-ui/lib/execution/types";
import {
	assertBoardSpecialistResponse,
	compactCopilotBoardReference,
} from "@flow-like/flow-like-ui/lib/flowpilot/board-chat-contract";
import type { FlowScriptApplyOrigin } from "@flow-like/flow-like-ui/lib/flowscript-apply-failure";
import {
	applyFlowScriptFromResponse,
	flowScriptFromResponse,
	scopedFlowScriptFromResponse,
} from "@flow-like/flow-like-ui/lib/flowscript/response";
import { requestTimeoutMs } from "@flow-like/flow-like-ui/lib/request-deadline";
import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import type { IChannelHandle } from "@flow-like/flow-like-ui/lib/schema/channel";
import type {
	ChatImage,
	CopilotScope,
	CopilotToolContext,
	UIActionContext,
	UnifiedChatMessage,
	UnifiedCopilotResponse,
} from "@flow-like/flow-like-ui/lib/schema/copilot";
import {
	type IBoardVersionInfo,
	normalizeBoardVersion,
} from "@flow-like/flow-like-ui/lib/schema/flow/board-version";
import type { IElementDemand } from "@flow-like/flow-like-ui/lib/schema/flow/element-demand";
import type { IOAuthRequiredError } from "@flow-like/flow-like-ui/state/backend-state/event-state";
import type { IPrerunBoardResponse } from "@flow-like/flow-like-ui/state/backend-state/types";
import { globalChatTransportRunId } from "@flow-like/flow-like-ui/state/global-chat/global-chat-run-control";
import { runGlobalChatTool } from "@flow-like/flow-like-ui/state/global-chat/global-chat-tool-registry";
import { dispatchSpecialistToolRequest } from "@flow-like/flow-like-ui/state/global-chat/global-chat-web-transport";
import { oauthConsentStore, oauthTokenStore } from "../oauth-db";
import { getOAuthApiBaseUrl, getOAuthService } from "../oauth-service";
import { prepareSessionRequest } from "../session-request";
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

function copilotRunChannelOf(data: string): IChannelHandle {
	let parsed: unknown;
	try {
		parsed = JSON.parse(data);
	} catch {
		throw new Error("Malformed Copilot run frame: payload is not valid JSON.");
	}
	const channel = (parsed as { channel?: unknown } | null)?.channel;
	if (!isChannelHandle(channel)) {
		throw new Error("Malformed Copilot run frame: channel is missing.");
	}
	return channel;
}

function copilotResponseOf(
	value: unknown,
	scope: CopilotScope,
): UnifiedCopilotResponse {
	if (scope === "Board") {
		assertBoardSpecialistResponse(value);
	} else if (!isRecord(value) || typeof value.message !== "string") {
		throw new Error(
			"Malformed final Copilot response: expected a response object with a message.",
		);
	}
	return value as UnifiedCopilotResponse;
}

interface CopilotRequestControl {
	abortController: AbortController;
	runChannel?: IChannelHandle;
	cancelRequested: boolean;
	cancelDelivery?: Promise<void>;
	cancelCompletion?: Promise<void>;
	resolveCancellation?: () => void;
	cancelGraceTimer?: ReturnType<typeof setTimeout>;
}

// The API sends the cancellation channel in the first SSE frame. A canceled request stays muted
// for this short grace period so an immediate Stop can still reach the server.
const COPILOT_RUN_CHANNEL_GRACE_MS = 1_000;

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

const RUN_OUTCOME_EVENTS = new Set([
	"completed",
	"execution_complete",
	"error",
]);

const SETTLED_ERROR_NAMES = new Set(["AbortError", "ExecutionDetachedError"]);

/**
 * A started run that failed, timed out, was stopped or lost its live updates has its outcome in its
 * logs. Board callers load them by run id, so only dispatch and transport failures reject.
 */
function endedWithOutcome(
	task: ExecutionTask | undefined,
	cause: unknown,
): boolean {
	if (!task?.runId) return false;
	const name = (cause as { name?: unknown } | null)?.name;
	return (
		(typeof name === "string" && SETTLED_ERROR_NAMES.has(name)) ||
		task.events.some((event) => RUN_OUTCOME_EVENTS.has(event.event_type))
	);
}

export class WebBoardState implements IBoardState {
	private readonly copilotRequestControls = new Map<
		string,
		CopilotRequestControl
	>();
	private readonly appIdByBoardId = new Map<string, string>();
	private readonly boardSync = new BoardSyncClient();
	/** The hub answered `/sync` as an unknown route; boards load whole from then on. */
	private boardSyncUnsupported = false;
	private readonly mediaProjector = new BoardMediaProjector();

	constructor(private readonly backend: WebBackendRef) {}

	private settleCopilotCancellation(control: CopilotRequestControl): void {
		if (control.cancelGraceTimer) {
			clearTimeout(control.cancelGraceTimer);
			control.cancelGraceTimer = undefined;
		}
		const resolve = control.resolveCancellation;
		control.resolveCancellation = undefined;
		resolve?.();
	}

	private finishCopilotCancellation(control: CopilotRequestControl): void {
		if (!control.runChannel) {
			control.cancelGraceTimer ??= setTimeout(() => {
				control.abortController.abort();
				this.settleCopilotCancellation(control);
			}, COPILOT_RUN_CHANNEL_GRACE_MS);
			return;
		}

		if (control.cancelGraceTimer) {
			clearTimeout(control.cancelGraceTimer);
			control.cancelGraceTimer = undefined;
		}
		control.cancelDelivery ??= cancelChannel(control.runChannel).catch(
			(error) => {
				console.warn("Failed to deliver Copilot cancellation:", error);
			},
		);
		control.abortController.abort();
		void control.cancelDelivery.then(() =>
			this.settleCopilotCancellation(control),
		);
	}

	private beginCopilotCancellation(
		control: CopilotRequestControl,
	): Promise<void> {
		control.cancelRequested = true;
		control.cancelCompletion ??= new Promise<void>((resolve) => {
			control.resolveCancellation = resolve;
		});
		this.finishCopilotCancellation(control);
		return control.cancelCompletion;
	}

	async getBoardFormat(appId: string): Promise<BoardFormatCapabilities> {
		return apiGet(`apps/${appId}/board/capabilities`, this.backend.auth);
	}

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

	async getBoardSummariesAuthoritative(
		appId: string,
		include?: IBoardSummaryInclude[],
	): Promise<IBoardSummary[]> {
		const query = include?.length ? `?include=${include.join(",")}` : "";
		return apiGet<IBoardSummary[]>(
			`apps/${appId}/board/summaries${query}`,
			this.backend.auth,
		);
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
		const board = await this.fetchBoard(appId, boardId, version);
		this.appIdByBoardId.set(boardId, appId);

		return this.currentDisplayBoard(appId, boardId, version, board);
	}

	private async fetchBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IBoard> {
		if (this.boardSyncUnsupported) {
			return this.getBoardAuthoritative(appId, boardId, version);
		}
		const params = version ? `?version=${version.join("_")}` : "";
		try {
			return await this.boardSync.sync(
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
		} catch (error) {
			// A hub that predates `/sync` has no such route; a coded 404 is a missing board.
			if (!isMissingRouteError(error)) throw error;
			this.boardSyncUnsupported = true;
			return this.getBoardAuthoritative(appId, boardId, version);
		}
	}

	async getBoardAuthoritative(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IBoard> {
		const params = version ? `?version=${version.join("_")}` : "";
		return apiGet<IBoard>(
			`apps/${appId}/board/${boardId}${params}`,
			this.backend.auth,
		);
	}

	async getBoardSummariesSnapshot(appId: string): Promise<IBoardSummary[]> {
		return apiGet<IBoardSummary[]>(
			`apps/${appId}/board/summaries`,
			this.backend.auth,
		);
	}

	async getBoardSnapshot(appId: string, boardId: string): Promise<IBoard> {
		return apiGet<IBoard>(`apps/${appId}/board/${boardId}`, this.backend.auth);
	}

	private async currentDisplayBoard(
		appId: string,
		boardId: string,
		version: [number, number, number] | undefined,
		board: IBoard,
	): Promise<IBoard> {
		let current = this.boardSync.peek(appId, boardId, version) ?? board;
		for (;;) {
			const display = await this.presignMediaComments(appId, boardId, current);
			const latest = this.boardSync.peek(appId, boardId, version);
			if (!latest || latest === current) return display;
			// A command can commit while media URLs load. Return its snapshot so
			// the older read cannot replace the command's result in the query cache.
			current = latest;
		}
	}

	private async presignMediaComments(
		appId: string,
		boardId: string,
		board: IBoard,
	): Promise<IBoard> {
		const files = this.mediaProjector.files(board);
		if (files.length === 0) return board;
		const scope = JSON.stringify([appId, boardId]);
		const fullPath = (filename: string) =>
			`apps/${appId}/upload/boards/${boardId}/${filename}`;
		try {
			const results = await apiPost<
				{ prefix: string; url?: string; error?: string }[]
			>(
				`apps/${appId}/data/download`,
				{ prefixes: files.map(fullPath) },
				this.backend.auth,
			);
			const byPrefix = new Map(
				results.flatMap((result) =>
					result.url ? [[result.prefix, result.url] as const] : [],
				),
			);
			const urls = new Map<string, string>();
			for (const file of files) {
				const url = byPrefix.get(fullPath(file));
				if (url) urls.set(file, url);
			}
			return this.mediaProjector.project(board, scope, urls);
		} catch (error) {
			console.warn("Failed to presign media comments:", error);
			return this.mediaProjector.project(board, scope, new Map());
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

	async getBoardVersionCurrent(
		appId: string,
		boardId: string,
	): Promise<FlowVersionState> {
		return parseFlowVersionState(
			await apiGet<unknown>(
				`apps/${encodeURIComponent(appId)}/board/${encodeURIComponent(boardId)}/version/current`,
				this.backend.auth,
			),
		);
	}

	async getBoardVersionInfos(
		appId: string,
		boardId: string,
	): Promise<IBoardVersionInfo[]> {
		try {
			return await apiGet<IBoardVersionInfo[]>(
				`apps/${appId}/board/${boardId}/version/info`,
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
		return withDeviceCommandBridge(
			{ appId, executionTarget: "remote" },
			cb,
			(bridgeCallback) =>
				this.executeBoardInternal(
					appId,
					boardId,
					payload,
					streamState,
					eventId,
					bridgeCallback,
					skipConsentCheck,
				),
		);
	}

	private async executeBoardInternal(
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
				const error: IOAuthRequiredError = Object.assign(
					new Error(
						`Missing OAuth authorization for: ${providersNeedingConsent.map((p) => p.name).join(", ")}`,
					),
					{
						missingProviders: providersNeedingConsent,
						isOAuthError: true as const,
					},
				);
				throw error;
			}
		} else {
			// Still need to check for missing tokens even if skipping consent
			if (oauthResult.missingProviders.length > 0) {
				const error: IOAuthRequiredError = Object.assign(
					new Error(
						`Missing OAuth tokens for: ${oauthResult.missingProviders.map((p) => p.name).join(", ")}`,
					),
					{
						missingProviders: oauthResult.missingProviders,
						isOAuthError: true as const,
					},
				);
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
		return withDeviceCommandBridge(
			{ appId, executionTarget: "remote" },
			cb,
			(bridgeCallback) =>
				this.executeBoardRemoteInternal(
					appId,
					boardId,
					payload,
					streamState,
					eventId,
					bridgeCallback,
					oauthTokens,
				),
		);
	}

	private async executeBoardRemoteInternal(
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
		const hub = getApiOrigin();
		const invokePath = `apps/${appId}/board/${boardId}/invoke`;
		const url = `${getApiBaseUrl()}/api/v1/${invokePath}`;
		const session = await prepareSessionRequest(
			{
				"Content-Type": "application/json",
				[BOARD_FORMAT_HEADER]: String(CURRENT_BOARD_FORMAT_VERSION),
			},
			this.backend.auth,
		);
		const body = JSON.stringify({
			node_id: payload.id,
			version: payload.version,
			payload: payload.payload,
			stream_state: streamState ?? true,
			oauth_tokens: oauthTokens,
			runtime_variables: payload.runtime_variables,
			profile_id: this.backend.profile?.id,
		});
		const taskId = crypto.randomUUID();
		const metadata = getExecutionMetadata(payload) ?? {
			taskId,
			streamId: taskId,
			scope: JSON.stringify([
				getApiOrigin(this.backend.profile),
				this.backend.profile?.id ?? "",
				this.backend.auth?.user?.profile?.sub ?? "local",
			]),
			appId,
			eventId: "",
			title: "Workflow",
			startedAt: new Date().toISOString(),
		};
		const runtime = await getExecutionRuntime(metadata.scope);
		try {
			return await runtime.execute(
				metadata,
				{
					url,
					headers: Object.fromEntries(session.headers.entries()),
					body,
					cancelUrl: `${getApiBaseUrl()}/api/v1/execution/run/{runId}`,
					timeoutMs: requestTimeoutMs(invokePath, "POST", body.length),
					realtime:
						this.backend.auth?.isAuthenticated &&
						this.backend.auth.user?.profile.sub
							? {
									scope: JSON.stringify([
										hub,
										this.backend.auth.user.profile.iss,
										this.backend.auth.user.profile.sub,
									]),
									url: `${getApiBaseUrl()}/api/v1/realtime/poll`,
									headers: Object.fromEntries(session.headers.entries()),
								}
							: undefined,
				},
				{
					onExecutionStart: eventId,
					onEvents: cb,
					onUnauthorized: session.onUnauthorized,
				},
			);
		} catch (cause) {
			if (endedWithOutcome(runtime.getTask(metadata.taskId), cause))
				return undefined;
			const response = cause as {
				status?: number;
				statusText?: string;
				headers?: Record<string, string>;
				body?: string;
			};
			if (response?.status === 401) session.onRejected?.();
			if (typeof response?.status === "number")
				throw apiResponseError(
					{
						status: response.status,
						statusText: response.statusText ?? "",
						headers: new Headers(response.headers),
					},
					response.body ?? "",
					url,
				);
			throw cause;
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

	async getRunPayload(
		appId: string,
		boardId: string,
		runId: string,
	): Promise<object | undefined> {
		try {
			return await apiGet<object | undefined>(
				`apps/${appId}/board/${boardId}/runs/${runId}/payload`,
				this.backend.auth,
			);
		} catch (error) {
			if (isMissingResourceError(error)) return undefined;
			throw error;
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

	async queryRunLogs(
		logMeta: ILogMetadata,
		query: ILogQuery,
		offset: number,
		limit: number,
	): Promise<ILog[]> {
		const logs = await apiPost<ILog[]>(
			`apps/${logMeta.app_id}/board/${logMeta.board_id}/logs/query`,
			{ run_id: logMeta.run_id, query, offset, limit },
			this.backend.auth,
		);
		return asArray(logs);
	}

	async countRunLogs(logMeta: ILogMetadata, query: ILogQuery): Promise<number> {
		const response = await apiPost<{ count?: number }>(
			`apps/${logMeta.app_id}/board/${logMeta.board_id}/logs/count`,
			{ run_id: logMeta.run_id, query },
			this.backend.auth,
		);
		return typeof response?.count === "number" ? response.count : 0;
	}

	async getRunLogSummary(
		logMeta: ILogMetadata,
	): Promise<IRunLogSummary | null> {
		const summary = await apiGet<IRunLogSummary | null>(
			`apps/${logMeta.app_id}/board/${logMeta.board_id}/logs/summary?run_id=${encodeURIComponent(logMeta.run_id)}`,
			this.backend.auth,
		);
		return isRecord(summary) ? (summary as IRunLogSummary) : null;
	}

	async undoBoard(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
		_options?: IBoardMutationOptions,
	): Promise<void> {
		await apiPatch(
			`apps/${appId}/board/${boardId}/undo`,
			{ commands },
			this.backend.auth,
		);
	}

	async redoBoard(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
		_options?: IBoardMutationOptions,
	): Promise<void> {
		await apiPatch(
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
				options.onBoard(
					await this.currentDisplayBoard(appId, boardId, undefined, board),
				);
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
		const response = await apiPost<unknown>(
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
		return applyFlowScriptFromResponse(response, boardId);
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
		return flowScriptFromResponse(response, boardId);
	}

	async renderFlowScript(
		appId: string,
		boardId: string,
		board: IBoard,
		anchors = true,
	): Promise<string> {
		const response = await apiPost<{ flowscript: string }>(
			`apps/${appId}/board/${boardId}/flowscript/render`,
			{ board, anchors },
			this.backend.auth,
		);
		return flowScriptFromResponse(response, boardId);
	}

	async getFlowScriptAuthoritative(
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
		return flowScriptFromResponse(response, boardId);
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
		return scopedFlowScriptFromResponse(response, boardId);
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
		return scopedFlowScriptFromResponse(response, boardId);
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
		return flowScriptFromResponse(response, boardId);
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
		readOnly?: boolean,
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
			scope === "Board" || scope === "Both"
				? (toolContext?.boardId ?? board?.id ?? runContext?.board_id)?.trim()
				: undefined;
		if (scope === "Board" && !contextBoardId) {
			throw new Error(
				"The board specialist requires a board id before it can run.",
			);
		}
		if (contextBoardId && board?.id && board.id !== contextBoardId) {
			throw new Error(
				"The board snapshot does not match the requested specialist target.",
			);
		}
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
			[BOARD_FORMAT_HEADER]: String(CURRENT_BOARD_FORMAT_VERSION),
		};
		if (onToken) {
			headers.Accept = "text/event-stream";
		}
		if (this.backend.auth?.user?.access_token) {
			headers.Authorization = `Bearer ${this.backend.auth.user.access_token}`;
		}

		const wantsStream = Boolean(onToken);
		const abortController = new AbortController();
		const requestControl: CopilotRequestControl = {
			abortController,
			cancelRequested: false,
		};
		if (requestId && requestControl) {
			const previous = this.copilotRequestControls.get(requestId);
			if (previous) {
				void this.beginCopilotCancellation(previous);
			}
			this.copilotRequestControls.set(requestId, requestControl);
		}
		try {
			const profileId = toolContext?.profileId ?? this.backend.profile?.id;
			const response = await fetch(url, {
				method: "POST",
				headers,
				signal: abortController.signal,
				body: JSON.stringify({
					scope,
					profile_id: profileId,
					// Both API versions reload the saved board. Keep the legacy envelope until
					// every server accepts board_id, without uploading the potentially large graph.
					board_id: contextBoardId,
					board: contextBoardId
						? compactCopilotBoardReference(contextBoardId)
						: undefined,
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
					read_only: readOnly ?? false,
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
				let finalReceived = false;
				let streamError: Error | undefined;
				let rejectStreamFailure!: (error: Error) => void;
				const streamFailure = new Promise<never>((_resolve, reject) => {
					rejectStreamFailure = reject;
				});
				// A tool delivery can fail while no SSE frames arrive. Wake the reader immediately.
				void streamFailure.catch(() => undefined);
				const failStream = (error: unknown) => {
					streamError ??=
						error instanceof Error ? error : new Error(String(error));
					rejectStreamFailure(streamError);
				};
				const onAbort = () =>
					failStream(
						new DOMException("Copilot run was cancelled", "AbortError"),
					);
				abortController.signal.addEventListener("abort", onAbort, {
					once: true,
				});
				if (abortController.signal.aborted) onAbort();
				// A nested Data Studio / Scout specialist runs server-side but owns no tools of its
				// own: they execute here, exactly like the orchestrator's, and their results go back
				// on the channel each request carries.
				const toolDispatches = new Set<Promise<void>>();
				const seenToolRequests = new Set<string>();

				const handleEvent = (rawEvent: string) => {
					if (finalReceived || streamError || !rawEvent.trim()) return;

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
					if (!data) {
						if (eventName === "final")
							failStream(new Error("Empty final Copilot response"));
						return;
					}

					if (eventName === "run") {
						try {
							requestControl.runChannel = copilotRunChannelOf(data);
							if (requestControl.cancelRequested) {
								this.finishCopilotCancellation(requestControl);
							}
						} catch (error) {
							failStream(error);
						}
						return;
					}
					if (requestControl?.cancelRequested) return;

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
							parentRequestId: toolContext?.parentRequestId,
							profileId,
							appId: contextAppId,
						}).catch((error) => {
							// The specialist is blocked on this request; failing the stream beats
							// leaving it to time out with no explanation.
							failStream(error);
						});
						toolDispatches.add(dispatch);
						void dispatch.finally(() => toolDispatches.delete(dispatch));
						return;
					}

					if (eventName === "final") {
						finalReceived = true;
						try {
							result = copilotResponseOf(JSON.parse(data), scope);
						} catch (error) {
							failStream(
								error instanceof SyntaxError
									? new Error("Failed to parse final Copilot response")
									: error,
							);
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
						failStream(new Error(message));
					}
				};

				const consumeBufferedEvents = () => {
					let separatorIndex = buffer.search(/\r?\n\r?\n/);
					while (separatorIndex !== -1 && !finalReceived) {
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

				try {
					while (!finalReceived) {
						const { done, value } = await Promise.race([
							reader.read(),
							streamFailure,
						]);
						if (done) break;
						buffer += decoder.decode(value, { stream: true });
						consumeBufferedEvents();
					}

					if (!finalReceived) {
						buffer += decoder.decode();
						if (buffer.trim()) handleEvent(buffer);
					}
					if (streamError) throw streamError;
					if (!result)
						throw new Error("Copilot stream ended without a final event");
					// Finish pending result deliveries, but let failures and cancellation wake the wait.
					if (toolDispatches.size > 0) {
						await Promise.race([
							Promise.all([...toolDispatches]),
							streamFailure,
						]);
					}
					if (streamError) throw streamError;
					return result;
				} catch (error) {
					if (!finalReceived)
						void this.beginCopilotCancellation(requestControl);
					throw error;
				} finally {
					abortController.signal.removeEventListener("abort", onAbort);
					void reader.cancel().catch(() => undefined);
					reader.releaseLock();
				}
			}

			const result = await response.json();
			if (requestControl.cancelRequested)
				throw new DOMException("Copilot run was cancelled", "AbortError");
			return copilotResponseOf(result, scope);
		} finally {
			if (requestControl?.cancelRequested && !requestControl.cancelDelivery) {
				this.settleCopilotCancellation(requestControl);
			}
			if (
				requestId &&
				this.copilotRequestControls.get(requestId) === requestControl
			) {
				this.copilotRequestControls.delete(requestId);
			}
		}
	}

	async cancelCopilotChat(requestId: string): Promise<void> {
		const control = this.copilotRequestControls.get(requestId);
		if (!control) return;
		await this.beginCopilotCancellation(control);
		if (this.copilotRequestControls.get(requestId) === control) {
			this.copilotRequestControls.delete(requestId);
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
