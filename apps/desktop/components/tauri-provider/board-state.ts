import {
	type ChatImage,
	type CopilotScope,
	type CopilotToolContext,
	type FlowIrCommitDisposition,
	type FlowIrCommitDispositionResult,
	type FlowIrCommitToken,
	type IApplyFlowIrCommitResponse,
	type IApplyFlowScriptResponse,
	type IBoard,
	type IBoardState,
	ICommentType,
	IConnectionMode,
	type IExecutionMode,
	type IExecutionStage,
	type IFlowScriptDiagnostic,
	type IGenericCommand,
	type IHub,
	type IIntercomEvent,
	type ILog,
	type ILogLevel,
	type ILogMetadata,
	type INode,
	type IOAuthProvider,
	type IPrerunBoardResponse,
	type IRunContext,
	type IRunPayload,
	type ISettingsProfile,
	type IVersionType,
	type ProgressToastData,
	type UIActionContext,
	type UnifiedChatMessage,
	type UnifiedCopilotResponse,
	checkOAuthTokens,
	extractOAuthRequirementsFromBoard,
	finishAllProgressToasts,
	injectDataFunction,
	isEqual,
	showProgressToast,
} from "@flow-like/flow-like-ui";
import type { IJwks, IRealtimeAccess } from "@flow-like/flow-like-ui";
import type { SurfaceComponent } from "@flow-like/flow-like-ui/components/a2ui/types";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import { flowPilotDebugLog } from "@flow-like/flow-like-ui/lib/flowpilot-debug";
import { normalizeBoardVersion } from "@flow-like/flow-like-ui/lib/schema/flow/board-version";
import { Channel, invoke } from "@tauri-apps/api/core";
import { isObject } from "lodash-es";
import { toast } from "sonner";
import { fetcher, streamFetcher } from "../../lib/api";
import {
	dispatchFlowNotificationEvent,
	dispatchFlowNotificationEvents,
} from "../../lib/flow-notification-events";
import { oauthConsentStore, oauthTokenStore } from "../../lib/oauth-db";
import { oauthService } from "../../lib/oauth-service";
import {
	ensureRpaSystemPermissions,
	requestRpaAutomationConsent,
} from "../rpa";
import type { TauriBackend } from "../tauri-provider";
import {
	getRemoteBoardSkipReason,
	shouldApplyRemoteBoard,
} from "./board-merge";
import {
	MAX_UNDO_REDO_SYNC_BODY_BYTES,
	OFFLINE_SYNC_COMMAND_MAX_AGE_MS,
	chunkCommandsForSync,
	evaluateBoardLineage,
	systemTimeToNanos,
} from "./command-sync";
import { mergeBoardOffThread } from "./board-sync";
import { resolveLocalFirstPrerun } from "./prerun-utils";

interface DiffEntry {
	path: string;
	local: any;
	remote: any;
}

const REMOTE_BOARD_APPLIED_EVENT = "flow:remote-board-applied";

// A burst of queued batches (chunked pushes all failing against the same
// outage) must surface as a single toast, not one per batch.
const QUEUED_EDITS_TOAST_DEBOUNCE_MS = 15_000;
let lastQueuedEditsToastAt = 0;

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

	hubCachePromise = fetch(`https://${hubUrl}/api/v1`)
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

function dispatchRemoteBoardApplied(appId: string, boardId: string) {
	if (typeof window === "undefined") {
		return;
	}

	window.dispatchEvent(
		new CustomEvent(REMOTE_BOARD_APPLIED_EVENT, {
			detail: {
				appId,
				boardId,
			},
		}),
	);
}

// Toast and Progress event handling for remote execution
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

const getDeepDifferences = (
	local: any,
	remote: any,
	path = "",
): DiffEntry[] => {
	const differences: DiffEntry[] = [];

	if (!isEqual(local, remote)) {
		if (!isObject(local) || !isObject(remote)) {
			differences.push({ path, local, remote });
		} else {
			const allKeys = new Set([
				...Object.keys(local || {}),
				...Object.keys(remote || {}),
			]);

			for (const key of allKeys) {
				const currentPath = path ? `${path}.${key}` : key;
				//@ts-ignore
				const localValue = local?.[key];
				//@ts-ignore
				const remoteValue = remote?.[key];

				if (!isEqual(localValue, remoteValue)) {
					differences.push(
						...getDeepDifferences(localValue, remoteValue, currentPath),
					);
				}
			}
		}
	}

	return differences;
};

// The full recursive diff below costs hundreds of ms on large boards — only
// run it when explicitly enabled for debugging board sync issues.
const isBoardSyncDebugEnabled = (): boolean => {
	try {
		return localStorage.getItem("flow-debug-board-sync") === "1";
	} catch {
		return false;
	}
};

const logBoardDifferences = (localBoard: IBoard, remoteBoard: IBoard) => {
	if (!isBoardSyncDebugEnabled()) return;
	const differences = getDeepDifferences(localBoard, remoteBoard);

	if (differences.length === 0) {
		console.log("No differences found between local and remote board");
		return;
	}

	console.log(
		`Found ${differences.length} differences between local and remote board:`,
	);
	console.table(
		differences.map((diff) => ({
			path: diff.path,
			localType: typeof diff.local,
			remoteType: typeof diff.remote,
			localValue:
				JSON.stringify(diff.local)?.slice(0, 100) +
				(JSON.stringify(diff.local)?.length > 100 ? "..." : ""),
			remoteValue:
				JSON.stringify(diff.remote)?.slice(0, 100) +
				(JSON.stringify(diff.remote)?.length > 100 ? "..." : ""),
		})),
	);

	differences.forEach((diff) => {
		console.groupCollapsed(`Path: ${diff.path}`);
		console.log("Local value:", diff.local);
		console.log("Remote value:", diff.remote);
		console.groupEnd();
	});
};
const getAppPackageCatalogNodes = (
	catalogNodes: INode[] | undefined,
): INode[] | undefined => {
	const packageNodes = catalogNodes?.filter((node) =>
		Boolean(node.wasm?.package_id),
	);

	return packageNodes?.length ? packageNodes : undefined;
};

const decodePinDefaultValue = (defaultValue?: number[] | null): unknown => {
	if (!defaultValue?.length) return undefined;

	try {
		const jsonString = new TextDecoder("utf-8").decode(
			new Uint8Array(defaultValue),
		);
		return JSON.parse(jsonString);
	} catch {
		return undefined;
	}
};

const summarizeBoardElementRefs = (board: IBoard) => {
	const summaries: Array<{
		nodeId: string;
		nodeName: string;
		pinId: string;
		pinName: string;
		defaultValue: unknown;
	}> = [];

	const collectNodePins = (node: INode) => {
		for (const pin of Object.values(node.pins ?? {})) {
			if (!pin.name.startsWith("element_ref")) continue;
			summaries.push({
				nodeId: node.id,
				nodeName: node.name,
				pinId: pin.id,
				pinName: pin.name,
				defaultValue: decodePinDefaultValue(pin.default_value),
			});
		}
	};

	for (const node of Object.values(board.nodes)) {
		collectNodePins(node);
	}

	for (const layer of Object.values(board.layers)) {
		for (const node of Object.values(layer.nodes)) {
			collectNodePins(node);
		}
	}

	return summaries;
};

export class BoardState implements IBoardState {
	constructor(private readonly backend: TauriBackend) {}

	private async syncRemoteAppPackages(
		appId: string,
	): Promise<Array<{ packageId: string; version: string }>> {
		const isOffline = await this.backend.isOffline(appId);

		if (
			isOffline ||
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.appState.listPackages ||
			!this.backend.appState.addPackage ||
			!this.backend.appState.removePackage
		) {
			return [];
		}

		try {
			const [remotePackages, localPackages] = await Promise.all([
				fetcher<Array<{ packageId: string; version: string }>>(
					this.backend.profile,
					`apps/${appId}/packages`,
					undefined,
					this.backend.auth,
				),
				this.backend.appState.listPackages(appId),
			]);

			const remotePackageMap = new Map(
				remotePackages.map((pkg) => [pkg.packageId, pkg.version]),
			);

			const syncTasks: Promise<void>[] = [];

			for (const [packageId, version] of remotePackageMap) {
				if (localPackages[packageId] === version) {
					continue;
				}

				syncTasks.push(
					this.backend.appState.addPackage(appId, packageId, version),
				);
			}

			for (const packageId of Object.keys(localPackages)) {
				if (remotePackageMap.has(packageId)) {
					continue;
				}

				syncTasks.push(this.backend.appState.removePackage(appId, packageId));
			}

			if (syncTasks.length > 0) {
				await Promise.all(syncTasks);
			}

			return remotePackages;
		} catch (error) {
			console.warn(
				"Failed to sync remote app packages into local catalog state:",
				error,
			);
			return [];
		}
	}

	async ensureRemoteAppPackagesInstalled(
		packages: Array<{ packageId: string; version: string }>,
		options: { forceReload?: boolean; throwOnError?: boolean } = {},
	): Promise<void> {
		if (!this.backend.registryState || packages.length === 0) {
			if (options.throwOnError && packages.length > 0) {
				throw new Error("Package registry is not available on this client.");
			}
			return;
		}

		try {
			const installedPackages =
				await this.backend.registryState.getInstalledPackages();
			const installedVersionMap = new Map(
				installedPackages.map((pkg) => [pkg.id, pkg.version]),
			);

			const installTasks = packages
				.filter(
					(pkg) =>
						options.forceReload ||
						installedVersionMap.get(pkg.packageId) !== pkg.version,
				)
				.map((pkg) =>
					this.backend.registryState.installPackage(pkg.packageId, pkg.version),
				);

			if (installTasks.length > 0) {
				await Promise.all(installTasks);
			}
		} catch (error) {
			console.warn(
				"Failed to install remote app packages into local registry:",
				error,
			);
			if (options.throwOnError) {
				throw new Error(
					getErrorMessage(
						error,
						"Failed to install remote app packages into local registry",
					),
				);
			}
		}
	}

	async ensureAppPackagesInstalledForExecution(appId: string): Promise<void> {
		const remotePackages = await this.syncRemoteAppPackages(appId);
		await this.ensureRemoteAppPackagesInstalled(remotePackages, {
			forceReload: true,
			throwOnError: true,
		});
	}

	async getBoards(appId: string): Promise<IBoard[]> {
		let boards: IBoard[] = await invoke("get_app_boards", {
			appId: appId,
		});
		boards = Array.from(new Map(boards.map((b) => [b.id, b])).values());

		const isOffline = await this.backend.isOffline(appId);

		if (isOffline) {
			return boards;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			console.warn(
				"Profile, auth or query client not set. Returning local boards only.",
			);
			return boards;
		}

		const promise = injectDataFunction(
			async () => {
				const mergedBoards = new Map<string, IBoard>();
				const remoteData = await fetcher<IBoard[]>(
					this.backend.profile!,
					`apps/${appId}/board`,
					{
						method: "GET",
					},
					this.backend.auth,
				);

				for (const board of boards) {
					mergedBoards.set(board.id, board);
				}

				for (const board of remoteData) {
					const localBoard = mergedBoards.get(board.id);

					if (localBoard && !shouldApplyRemoteBoard(board, localBoard)) {
						console.warn(
							"Skipping stale or incomplete remote board during board list sync:",
							{
								boardId: board.id,
								skipReason: getRemoteBoardSkipReason(board, localBoard),
								localPageIds: localBoard.page_ids,
								remotePageIds: board.page_ids,
								localUpdatedAt: localBoard.updated_at,
								remoteUpdatedAt: board.updated_at,
							},
						);
						continue;
					}

					if (localBoard) {
						const pendingSync = await this.backend.getOfflineSyncCommands(
							appId,
							board.id,
						);
						if (pendingSync.length > 0) {
							// Local edits are still queued for the server; the remote snapshot
							// predates them and applying it would clobber the local content.
							console.warn(
								"Skipping remote board with pending offline sync commands:",
								{ boardId: board.id, pendingBatches: pendingSync.length },
							);
							continue;
						}

						if (
							!(await this.lineageAllowsRemoteApply(appId, board.id, board))
						) {
							continue;
						}
					}

					const { merged, changed } = await mergeBoardOffThread(
						board,
						localBoard,
					);

					if (changed) {
						console.log("Board data changed, updating local state:");
						await invoke("upsert_board", {
							appId: appId,
							boardId: merged.id,
							name: merged.name,
							description: merged.description,
							logLevel: merged.log_level,
							stage: merged.stage,
							executionMode: merged.execution_mode,
							boardData: merged,
						});
						await this.recordAppliedRemoteLineage(appId, board.id, board);
					}

					// Keep the local reference when content is unchanged so downstream
					// deep-equality checks short-circuit on identity.
					mergedBoards.set(board.id, changed ? merged : (localBoard ?? merged));
				}

				return Array.from(mergedBoards.values());
			},
			this,
			this.backend.queryClient,
			this.getBoards,
			[appId],
			[],
			boards,
		);

		this.backend.backgroundTaskHandler(promise);

		return boards;
	}
	async getCatalog(appId: string): Promise<INode[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline && this.backend.profile && this.backend.auth) {
			try {
				return await fetcher<INode[]>(
					this.backend.profile,
					`apps/${appId}/nodes`,
					{ method: "GET" },
					this.backend.auth,
				);
			} catch (error) {
				console.warn(
					"Failed to fetch remote app catalog, falling back to local catalog:",
					error,
				);
			}
		}

		const remotePackages = await this.syncRemoteAppPackages(appId);
		await this.ensureRemoteAppPackagesInstalled(remotePackages);
		const nodes: INode[] = await invoke("get_catalog", { appId });
		return nodes;
	}
	async getBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		forceFresh?: boolean,
	): Promise<IBoard> {
		let board: IBoard;
		try {
			board = await invoke("get_board", {
				appId: appId,
				boardId: boardId,
				version: version,
			});
		} catch {
			const isOffline = await this.backend.isOffline(appId);
			if (isOffline || !this.backend.profile || !this.backend.auth) {
				throw new Error(`Board not found: ${boardId}`);
			}
			let url = `apps/${appId}/board/${boardId}`;
			if (version) {
				url += `?version=${version.join("_")}`;
			}
			const remoteData = await fetcher<IBoard>(
				this.backend.profile,
				url,
				{ method: "GET" },
				this.backend.auth,
			);
			if (typeof version === "undefined") {
				await invoke("upsert_board", {
					appId: appId,
					boardId: boardId,
					name: remoteData.name,
					description: remoteData.description,
					logLevel: remoteData.log_level,
					stage: remoteData.stage,
					executionMode: remoteData.execution_mode,
					boardData: remoteData,
				}).catch((e: unknown) => {
					console.warn(
						"[BoardState] Failed to persist remote board locally:",
						e,
					);
				});
			}
			if (typeof version === "undefined") {
				await this.recordAppliedRemoteLineage(appId, boardId, remoteData);
				dispatchRemoteBoardApplied(appId, boardId);
			}
			return remoteData;
		}

		const isOffline = await this.backend.isOffline(appId);

		// Presign media comments for display
		await this.presignMediaComments(appId, boardId, board, isOffline);

		if (typeof version !== "undefined") {
			return board;
		}

		if (
			isOffline ||
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			return board;
		}

		// When forceFresh is set, synchronously fetch from remote and persist
		// before returning. This ensures the board in local storage is up-to-date
		// before execution begins (used on the /use page and execution paths).
		if (forceFresh) {
			try {
				const pendingSync = await this.backend.getOfflineSyncCommands(
					appId,
					boardId,
				);
				if (pendingSync.length > 0) {
					// Local edits are still queued for the server; the remote snapshot
					// predates them, so the local board is the fresher one.
					console.warn(
						"[BoardState] forceFresh: local board has pending offline sync commands, skipping remote overwrite:",
						{ boardId, pendingBatches: pendingSync.length },
					);
					return board;
				}

				const url = `apps/${appId}/board/${boardId}`;
				const remoteData = await fetcher<IBoard>(
					this.backend.profile,
					url,
					{ method: "GET" },
					this.backend.auth,
				);

				if (remoteData) {
					if (
						!(await this.lineageAllowsRemoteApply(appId, boardId, remoteData))
					) {
						return board;
					}

					const { merged, changed } = await mergeBoardOffThread(
						remoteData,
						board,
					);
					if (changed && typeof version === "undefined") {
						console.log("[BoardState] forceFresh: updating local board:", {
							boardId,
						});
						await invoke("upsert_board", {
							appId: appId,
							boardId: boardId,
							name: merged.name,
							description: merged.description,
							logLevel: merged.log_level,
							stage: merged.stage,
							executionMode: merged.execution_mode,
							boardData: merged,
						});
						await this.recordAppliedRemoteLineage(appId, boardId, remoteData);
						dispatchRemoteBoardApplied(appId, boardId);

						if (this.backend.queryClient) {
							const queryKey = [
								this.getBoard.name || "backendFn",
								appId,
								boardId,
								version,
							].filter((arg) => typeof arg !== "undefined");
							this.backend.queryClient.setQueryData(queryKey, merged);
						}
						return merged;
					}
					return board;
				}
			} catch (e) {
				console.warn(
					"[BoardState] forceFresh sync failed, using local board:",
					e,
				);
			}
			return board;
		}

		const promise = injectDataFunction(
			async () => {
				const { failed: drainFailed } = await this.drainOfflineSyncQueue(
					appId,
					boardId,
				);

				if (drainFailed) {
					// Local edits are not on the server yet. Applying the remote snapshot
					// now would clobber them with a board that predates the queued batch.
					return board;
				}

				const remoteData = await fetcher<IBoard>(
					this.backend.profile!,
					`apps/${appId}/board/${boardId}`,
					{
						method: "GET",
					},
					this.backend.auth,
				);

				if (!remoteData) {
					throw new Error("Failed to fetch board data");
				}

				if (!shouldApplyRemoteBoard(remoteData, board)) {
					console.warn(
						"Skipping stale or incomplete remote board during board sync:",
						{
							boardId,
							skipReason: getRemoteBoardSkipReason(remoteData, board),
							localPageIds: board.page_ids,
							remotePageIds: remoteData.page_ids,
							localUpdatedAt: board.updated_at,
							remoteUpdatedAt: remoteData.updated_at,
							localElementRefs: summarizeBoardElementRefs(board),
							remoteElementRefs: summarizeBoardElementRefs(remoteData),
						},
					);
					return board;
				}

				if (
					!(await this.lineageAllowsRemoteApply(appId, boardId, remoteData))
				) {
					return board;
				}

				const { merged, changed } = await mergeBoardOffThread(
					remoteData,
					board,
				);

				if (changed && typeof version === "undefined") {
					console.log("Board Missmatch, updating local state:");

					logBoardDifferences(board, merged);

					await invoke("upsert_board", {
						appId: appId,
						boardId: boardId,
						name: merged.name,
						description: merged.description,
						logLevel: merged.log_level,
						stage: merged.stage,
						executionMode: merged.execution_mode,
						boardData: merged,
					});
					await this.recordAppliedRemoteLineage(appId, boardId, remoteData);
					dispatchRemoteBoardApplied(appId, boardId);
					return merged;
				}

				console.log("Board data is up to date, no update needed.");
				// Same reference → the caller's deep-equality check short-circuits and
				// the query cache keeps identity, so the board is not re-parsed.
				return board;
			},
			this,
			this.backend.queryClient,
			this.getBoard,
			[appId, boardId, version],
			[],
			board,
		);

		this.backend.backgroundTaskHandler(promise);

		return board;
	}

	async getRealtimeAccess(
		appId: string,
		boardId: string,
	): Promise<IRealtimeAccess> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) throw new Error("Realtime is unavailable offline");
		if (!this.backend.profile || !this.backend.auth)
			throw new Error("Missing auth/profile for realtime access");

		const access = await fetcher<IRealtimeAccess>(
			this.backend.profile,
			`apps/${appId}/board/${boardId}/realtime`,
			{ method: "POST" },
			this.backend.auth,
		);

		return access;
	}

	async getRealtimeJwks(appId: string, boardId: string): Promise<IJwks> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) throw new Error("Realtime is unavailable offline");
		if (!this.backend.profile || !this.backend.auth)
			throw new Error("Missing auth/profile for realtime JWKS");

		const jwks = await fetcher<IJwks>(
			this.backend.profile,
			`apps/${appId}/board/${boardId}/realtime`,
			{ method: "GET" },
			this.backend.auth,
		);
		return jwks;
	}

	private async presignMediaComments(
		appId: string,
		boardId: string,
		board: IBoard,
		isOffline: boolean,
	): Promise<void> {
		const mediaComments = Object.values(board.comments).filter(
			(comment) =>
				comment.comment_type === ICommentType.Image ||
				comment.comment_type === ICommentType.Video,
		);

		// Collect layer media comments as well
		const layerMediaComments: { comment: any; layer: any }[] = [];
		for (const layer of Object.values(board.layers)) {
			for (const comment of Object.values(layer.comments)) {
				if (
					comment.comment_type === ICommentType.Image ||
					comment.comment_type === ICommentType.Video
				) {
					layerMediaComments.push({ comment, layer });
				}
			}
		}

		if (mediaComments.length === 0 && layerMediaComments.length === 0) return;

		if (isOffline) {
			// For offline mode, use Tauri's storage_get to get file URLs
			try {
				const prefixes = [
					...mediaComments.map((c) => `boards/${boardId}/${c.content}`),
					...layerMediaComments.map(
						({ comment }) => `boards/${boardId}/${comment.content}`,
					),
				];

				const results = await invoke<{ prefix: string; url?: string }[]>(
					"storage_get",
					{ appId, prefixes },
				);

				const urlMap = new Map(
					results.filter((r) => r.url).map((r) => [r.prefix, r.url as string]),
				);

				for (const comment of mediaComments) {
					const prefix = `boards/${boardId}/${comment.content}`;
					const url = urlMap.get(prefix);
					if (url) {
						(comment as any).presigned_url = url;
					}
				}

				for (const { comment } of layerMediaComments) {
					const prefix = `boards/${boardId}/${comment.content}`;
					const url = urlMap.get(prefix);
					if (url) {
						(comment as any).presigned_url = url;
					}
				}
			} catch (error) {
				console.warn("Failed to presign media comments (offline):", error);
			}
		} else if (this.backend.profile && this.backend.auth) {
			// For online mode, use the API to get presigned URLs
			try {
				const prefixes = [
					...mediaComments.map((c) => `boards/${boardId}/${c.content}`),
					...layerMediaComments.map(
						({ comment }) => `boards/${boardId}/${comment.content}`,
					),
				];

				const results = await fetcher<{ prefix: string; url?: string }[]>(
					this.backend.profile,
					`apps/${appId}/data/download`,
					{
						method: "POST",
						body: JSON.stringify({ prefixes }),
					},
					this.backend.auth,
				);

				const urlMap = new Map(
					results.filter((r) => r.url).map((r) => [r.prefix, r.url as string]),
				);

				for (const comment of mediaComments) {
					const prefix = `boards/${boardId}/${comment.content}`;
					const url = urlMap.get(prefix);
					if (url) {
						(comment as any).presigned_url = url;
					}
				}

				for (const { comment } of layerMediaComments) {
					const prefix = `boards/${boardId}/${comment.content}`;
					const url = urlMap.get(prefix);
					if (url) {
						(comment as any).presigned_url = url;
					}
				}
			} catch (error) {
				console.warn("Failed to presign media comments (online):", error);
			}
		}
	}

	async createBoardVersion(
		appId: string,
		boardId: string,
		versionType: IVersionType,
	): Promise<[number, number, number]> {
		const newVersion: [number, number, number] = await invoke(
			"create_board_version",
			{
				appId: appId,
				boardId: boardId,
				versionType: versionType,
			},
		);

		const isOffline = await this.backend.isOffline(appId);
		if (
			isOffline ||
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			return newVersion;
		}

		const promise = injectDataFunction(
			async () => {
				const remoteData = await fetcher<[number, number, number]>(
					this.backend.profile!,
					`apps/${appId}/board/${boardId}`,
					{
						method: "PATCH",
						body: JSON.stringify({
							version_type: versionType,
						}),
					},
					this.backend.auth,
				);

				return remoteData;
			},
			this,
			this.backend.queryClient,
			this.createBoardVersion,
			[appId, boardId, versionType],
			[],
			newVersion,
		);

		this.backend.backgroundTaskHandler(promise);

		return newVersion;
	}
	async getBoardVersions(
		appId: string,
		boardId: string,
	): Promise<[number, number, number][]> {
		const boardVersions: [number, number, number][] = await invoke(
			"get_board_versions",
			{
				appId: appId,
				boardId: boardId,
			},
		);

		const isOffline = await this.backend.isOffline(appId);
		if (
			isOffline ||
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			return boardVersions;
		}

		const promise = injectDataFunction(
			async () => {
				const remoteData = await fetcher<[number, number, number][]>(
					this.backend.profile!,
					`apps/${appId}/board/${boardId}/version`,
					{
						method: "GET",
					},
					this.backend.auth,
				);

				return remoteData;
			},
			this,
			this.backend.queryClient,
			this.getBoardVersions,
			[appId, boardId],
			[],
			boardVersions,
		);

		this.backend.backgroundTaskHandler(promise);

		return boardVersions;
	}
	async deleteBoard(appId: string, boardId: string): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) {
			await invoke("delete_app_board", {
				appId: appId,
				boardId: boardId,
			});
			return;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Profile, auth or query client not set. Cannot delete board.",
			);
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/board/${boardId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);

		await invoke("delete_app_board", {
			appId: appId,
			boardId: boardId,
		});
	}
	async getOpenBoards(): Promise<[string, string, string][]> {
		const boards: [string, string, string][] = await invoke("get_open_boards");
		return boards;
	}
	async getBoardSettings(): Promise<IConnectionMode> {
		const profile: ISettingsProfile = await invoke("get_current_profile");
		return (
			profile?.hub_profile.settings?.connection_mode ?? IConnectionMode.Default
		);
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
		// Check if board requires local execution (computer automation)
		// and verify RPA permissions before proceeding
		const board = await this.getBoard(
			appId,
			boardId,
			normalizeBoardVersion(payload.version),
			true,
		);
		await this.ensureAppPackagesInstalledForExecution(appId);
		const { requires_local_execution } =
			extractOAuthRequirementsFromBoard(board);

		console.log("[BoardState] executeBoard board summary:", {
			boardId,
			pageIds: board.page_ids,
			updatedAt: board.updated_at,
			elementRefs: summarizeBoardElementRefs(board),
		});

		if (requires_local_execution) {
			try {
				const approved = await requestRpaAutomationConsent({
					appId,
					boardId,
					context: "execution",
				});
				if (!approved) {
					const error = new Error(
						"Computer automation was not approved for this board.",
					) as Error & { isRpaConsentError?: boolean };
					error.isRpaConsentError = true;
					throw error;
				}

				const permissionsGranted = await ensureRpaSystemPermissions({
					appId,
					boardId,
				});
				if (!permissionsGranted) {
					const error = new Error(
						"RPA system permissions were not granted.",
					) as Error & { isRpaPermissionDeclined?: boolean };
					error.isRpaPermissionDeclined = true;
					throw error;
				}
			} catch (e) {
				const rpaError = e as {
					isRpaConsentError?: boolean;
					isRpaPermissionDeclined?: boolean;
					isRpaPermissionError?: boolean;
				};
				if (rpaError.isRpaPermissionError) throw e;
				if (rpaError.isRpaConsentError) throw e;
				if (rpaError.isRpaPermissionDeclined) throw e;
				console.warn("Failed to check RPA permissions:", e);
				const error = new Error(
					"Failed to verify RPA permissions. This workflow cannot run without a successful permission check.",
				);
				(error as any).isRpaPermissionError = true;
				(error as any).cause = e;
				throw error;
			}
		}

		const channel = new Channel<IIntercomEvent[]>();
		let foundRunId = false;

		const isOffline = await this.backend.isOffline(appId);
		let credentials = undefined;

		if (!isOffline && this.backend.auth && this.backend.profile) {
			try {
				credentials = await fetcher(
					this.backend.profile,
					`apps/${appId}/invoke/presign`,
					{
						method: "GET",
					},
					this.backend.auth,
				);
			} catch (e) {
				console.warn(e);
			}
		}

		// Collect OAuth tokens from board nodes using shared helper
		let oauthTokens:
			| Record<
					string,
					{
						access_token: string;
						refresh_token?: string;
						expires_at?: number;
						token_type?: string;
					}
			  >
			| undefined;
		const hub = await getHubConfig(this.backend.profile);
		const oauthResult = await checkOAuthTokens(board, oauthTokenStore, hub, {
			refreshToken: oauthService.refreshToken.bind(oauthService),
		});

		console.log("[OAuth] Board check result:", {
			requiredProviders: oauthResult.requiredProviders.map((p) => p.id),
			missingProviders: oauthResult.missingProviders.map((p) => p.id),
			hasTokens: Object.keys(oauthResult.tokens),
			skipConsentCheck,
		});

		// Check consent for providers that have tokens but might not have consent for this app
		// Skip this check if explicitly told to (e.g., after user consented in dialog)
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
				// Throw a special error that the UI can catch to show consent dialog
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

		if (Object.keys(oauthResult.tokens).length > 0) {
			oauthTokens = oauthResult.tokens;
		}

		channel.onmessage = (events: IIntercomEvent[]) => {
			if (!foundRunId && events.length > 0 && eventId) {
				const runId_event = events.find(
					(event) => event.event_type === "run_initiated",
				);

				if (runId_event) {
					const runId = runId_event.payload.run_id;
					eventId(runId);
					foundRunId = true;
				}
			}

			dispatchFlowNotificationEvents(events, appId);

			if (cb) cb(events);
		};

		const token = this.backend.auth?.user?.access_token;

		let metadata: ILogMetadata | undefined;
		try {
			metadata = await invoke("execute_board", {
				appId: appId,
				boardId: boardId,
				payload: payload,
				version: normalizeBoardVersion(payload.version),
				events: channel,
				streamState: streamState,
				credentials,
				token,
				oauthTokens,
			});

			// Yield to the event loop so any pending channel messages
			// (A2UI updates, etc.) are delivered before we finish.
			await new Promise<void>((resolve) => setTimeout(resolve, 0));
			finishAllProgressToasts(true);
		} catch (error) {
			finishAllProgressToasts(false);
			throw error;
		}

		return metadata;
	}

	async executeBoardRemote(
		appId: string,
		boardId: string,
		payload: IRunPayload,
		streamState?: boolean,
		eventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
	): Promise<ILogMetadata | undefined> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile and auth required for remote execution");
		}

		let closed = false;
		let foundRunId = false;

		await streamFetcher<IIntercomEvent>(
			this.backend.profile,
			`apps/${appId}/board/${boardId}/invoke`,
			{
				method: "POST",
				body: JSON.stringify({
					node_id: payload.id,
					version: payload.version,
					payload: payload.payload,
					token: this.backend.auth.user?.access_token,
					stream_state: streamState ?? true,
					runtime_variables: payload.runtime_variables,
					profile_id: this.backend.profile?.id,
				}),
			},
			this.backend.auth,
			(event: IIntercomEvent) => {
				if (closed) {
					console.warn("Stream closed, ignoring event");
					return;
				}

				// Handle run_initiated event to get run ID
				if (!foundRunId && eventId && event.event_type === "run_initiated") {
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

				if (event.event_type === "flow_notification") {
					dispatchFlowNotificationEvent(event, appId);
				}

				// Check for terminal events and finish progress toasts
				if (event.event_type === "completed") {
					finishAllProgressToasts(true);
				} else if (event.event_type === "error") {
					finishAllProgressToasts(false);
				}

				// Forward event to callback as array (consistent with local execution)
				if (cb) cb([event]);
				else {
					console.log("UNDELIVERED Received event:", event);
				}
			},
		);

		closed = true;
		finishAllProgressToasts(true);
		// Full metadata will be fetched separately by the caller
		return undefined;
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
		let localRuns: ILogMetadata[] = [];
		// Fetch local runs
		try {
			localRuns = await invoke("list_runs", {
				appId: appId,
				boardId: boardId,
				nodeId: nodeId,
				from: from,
				to: to,
				status: status,
				limit: limit,
				offset: offset,
				lastMeta: lastMeta,
			});
		} catch (e) {}

		// Mark local runs
		for (const run of localRuns) {
			run.is_remote = false;
		}

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) {
			return localRuns;
		}

		// Try to fetch remote runs for online apps.
		let remoteRuns: ILogMetadata[] = [];
		if (this.backend.profile && this.backend.auth) {
			try {
				const params = new URLSearchParams();
				if (nodeId) params.set("node_id", nodeId);
				if (from) params.set("from", from.toString());
				if (to) params.set("to", to.toString());
				if (status !== undefined) params.set("status", status.toString());
				if (limit) params.set("limit", limit.toString());
				if (offset) params.set("offset", offset.toString());
				if (includeNodes) params.set("include_nodes", "true");

				const queryString = params.toString();
				const path = `apps/${appId}/board/${boardId}/runs${queryString ? `?${queryString}` : ""}`;

				const response = await fetcher<ILogMetadata[]>(
					this.backend.profile,
					path,
					{ method: "GET" },
					this.backend.auth,
				);

				remoteRuns = response ?? [];

				for (const run of remoteRuns) {
					run.is_remote = true;
				}
			} catch (e) {
				console.warn("Failed to fetch remote runs:", e);
			}
		}

		// Merge and deduplicate by run_id, preferring local runs
		const runMap = new Map<string, ILogMetadata>();
		for (const run of remoteRuns) {
			runMap.set(run.run_id, run);
		}
		for (const run of localRuns) {
			runMap.set(run.run_id, run);
		}

		// Sort by start time descending (newest first)
		const merged = Array.from(runMap.values()).sort(
			(a, b) => b.start - a.start,
		);

		return merged;
	}

	async queryRun(
		logMeta: ILogMetadata,
		query: string,
		offset?: number,
		limit?: number,
	): Promise<ILog[]> {
		// Check if this is a remote run - fetch from API
		if (logMeta.is_remote && this.backend.profile && this.backend.auth) {
			try {
				const params = new URLSearchParams();
				params.set("run_id", logMeta.run_id);
				if (query) params.set("query", query);
				if (limit !== undefined) params.set("limit", limit.toString());
				if (offset !== undefined) params.set("offset", offset.toString());

				const path = `apps/${logMeta.app_id}/board/${logMeta.board_id}/logs?${params.toString()}`;
				const logs = await fetcher<ILog[]>(
					this.backend.profile,
					path,
					{ method: "GET" },
					this.backend.auth,
				);
				return logs ?? [];
			} catch (e) {
				console.error("Failed to fetch remote logs:", e);
				return [];
			}
		}

		// Local run - use Tauri invoke
		const runs: ILog[] = await invoke("query_run", {
			logMeta: logMeta,
			query: query,
			limit: limit,
			offset: offset,
		});
		return runs;
	}

	async undoBoard(appId: string, boardId: string, commands: IGenericCommand[]) {
		const isOffline = await this.backend.isOffline(appId);

		if (isOffline) {
			await invoke("undo_board", {
				appId: appId,
				boardId: boardId,
				commands: commands,
			});
			return;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			toast.error("Undo only works when you are online.");
			throw new Error(
				"Profile, auth or query client not set. Cannot push board update.",
			);
		}

		// Undo must ship as a single request — a chunked/partial undo would
		// diverge the board — so fail fast instead of hitting a raw HTTP 413.
		const body = JSON.stringify({ commands: commands });
		if (body.length > MAX_UNDO_REDO_SYNC_BODY_BYTES) {
			toast.error("Undo batch too large to sync. Undo in smaller steps.");
			throw new Error(
				`Undo batch of ${commands.length} commands (${body.length} bytes) exceeds the ${MAX_UNDO_REDO_SYNC_BODY_BYTES} byte sync limit`,
			);
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/board/${boardId}/undo`,
			{
				method: "PATCH",
				body,
			},
			this.backend.auth,
		);
	}
	async redoBoard(appId: string, boardId: string, commands: IGenericCommand[]) {
		const isOffline = await this.backend.isOffline(appId);

		if (isOffline) {
			await invoke("redo_board", {
				appId: appId,
				boardId: boardId,
				commands: commands,
			});
			return;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			toast.error("Undo only works when you are online.");
			throw new Error(
				"Profile, auth or query client not set. Cannot push board update.",
			);
		}

		// Redo must ship as a single request — a chunked/partial redo would
		// diverge the board — so fail fast instead of hitting a raw HTTP 413.
		const body = JSON.stringify({ commands: commands });
		if (body.length > MAX_UNDO_REDO_SYNC_BODY_BYTES) {
			toast.error("Redo batch too large to sync. Redo in smaller steps.");
			throw new Error(
				`Redo batch of ${commands.length} commands (${body.length} bytes) exceeds the ${MAX_UNDO_REDO_SYNC_BODY_BYTES} byte sync limit`,
			);
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/board/${boardId}/redo`,
			{
				method: "PATCH",
				body,
			},
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
	) {
		const isOffline = await this.backend.isOffline(appId);

		if (isOffline) {
			await invoke("upsert_board", {
				appId: appId,
				boardId: boardId,
				name: name,
				description: description,
				logLevel: logLevel,
				stage: stage,
				executionMode: executionMode,
				template: template,
			});
			return;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Profile, auth or query client not set. Cannot push board update.",
			);
		}

		const boardUpdate = await fetcher<{
			id: string;
			updated_at?: IBoard["updated_at"];
		}>(
			this.backend.profile,
			`apps/${appId}/board/${boardId}`,
			{
				method: "PUT",
				body: JSON.stringify({
					name: name,
					description: description,
					log_level: logLevel,
					stage: stage,
					execution_mode: executionMode,
					template: template,
				}),
			},
			this.backend.auth,
		);

		if (!boardUpdate?.id) {
			throw new Error("Failed to update board");
		}

		// Keep the authoritative remote write immediately readable by desktop callers. Previously an
		// online upsert never populated the local store; getBoards() therefore returned [] while its
		// remote sync ran in the background. A create_app -> flowpilot_board sequence interpreted that
		// empty snapshot as "no board", created a duplicate board, and could then hit a propagation 404.
		try {
			// Older deployed APIs return only `{ id }`. Use an intentionally old revision for that
			// compatibility path so the transient readiness cache can never outrank the authoritative
			// remote board during the next sync.
			const authoritativeUpdatedAt = boardUpdate.updated_at ?? {
				secs_since_epoch: 0,
				nanos_since_epoch: 0,
			};
			await invoke("upsert_board", {
				appId,
				boardId,
				name,
				description,
				logLevel,
				stage,
				executionMode,
				template,
				authoritativeUpdatedAt,
			});
			// Only advance the lineage once the local cache holds this revision;
			// otherwise a refused remote echo could block cache recovery.
			if (boardUpdate.updated_at) {
				await this.recordAppliedRemoteLineage(appId, boardId, {
					updated_at: boardUpdate.updated_at,
				});
			}
		} catch (error) {
			// The remote write already succeeded. Do not turn a cache failure into a failed
			// create_app response (and a duplicate retry); the readiness path can still fetch it.
			console.warn("Failed to cache the remote board locally:", error);
		}
	}

	async closeBoard(boardId: string) {
		await invoke("close_board", {
			boardId: boardId,
		});
	}

	/**
	 * Lineage guard on top of the existing updated_at checks: refuse a remote
	 * board that is not strictly newer than the last revision this client
	 * applied or pushed past. A cache miss or lookup failure falls back to the
	 * existing guards, so this can only add refusals.
	 */
	private async lineageAllowsRemoteApply(
		appId: string,
		boardId: string,
		remoteBoard: IBoard,
	): Promise<boolean> {
		try {
			const cachedLineageNs = await this.backend.getBoardLineage(
				appId,
				boardId,
			);
			const decision = evaluateBoardLineage(
				systemTimeToNanos(remoteBoard.updated_at),
				cachedLineageNs,
			);
			if (!decision.apply) {
				console.warn("Skipping remote board due to sync lineage guard:", {
					boardId,
					skipReason: decision.refusalReason,
					remoteUpdatedAt: remoteBoard.updated_at,
					cachedLineageNs,
				});
			}
			return decision.apply;
		} catch (error) {
			console.warn(
				"Board lineage lookup failed; falling back to existing sync guards:",
				error,
			);
			return true;
		}
	}

	private async recordAppliedRemoteLineage(
		appId: string,
		boardId: string,
		remoteBoard: Pick<IBoard, "updated_at">,
	): Promise<void> {
		try {
			await this.backend.recordBoardLineage(
				appId,
				boardId,
				systemTimeToNanos(remoteBoard.updated_at),
			);
		} catch (error) {
			console.warn("Failed to record board sync lineage:", error);
		}
	}

	/**
	 * After a successful command push the server holds at least the revision of
	 * the board snapshot this client is working on. The push response carries no
	 * timestamp, so record the cached board's updated_at (max-merge — it never
	 * moves the lineage backwards).
	 */
	private async recordLineageAfterPush(
		appId: string,
		boardId: string,
	): Promise<void> {
		const cachedBoard = this.backend.queryClient?.getQueryData<IBoard>([
			this.getBoard.name || "backendFn",
			appId,
			boardId,
		]);
		if (!cachedBoard?.updated_at) return;
		await this.recordAppliedRemoteLineage(appId, boardId, cachedBoard);
	}

	private notifyEditsQueued(appId: string, boardId: string): void {
		const now = Date.now();
		if (now - lastQueuedEditsToastAt < QUEUED_EDITS_TOAST_DEBOUNCE_MS) return;
		lastQueuedEditsToastAt = now;

		toast.warning(
			"Server sync failed — your edits are queued and will retry on the next board load.",
			{
				action: {
					label: "Retry now",
					onClick: () => {
						void this.retryOfflineSync(appId, boardId);
					},
				},
			},
		);
	}

	/**
	 * Ordered, chunked drain of the offline sync queue. Stops on the first
	 * failed batch so a later batch can never overtake an earlier one. Shared by
	 * getBoard's background sync and manual retries so the two paths cannot
	 * diverge.
	 */
	private async drainOfflineSyncQueue(
		appId: string,
		boardId: string,
	): Promise<{ failed: boolean; pushedBatches: number }> {
		const unsyncedCommands = await this.backend.getOfflineSyncCommands(
			appId,
			boardId,
		);
		let failed = false;
		let pushedBatches = 0;

		for (const commandSync of unsyncedCommands) {
			// Replaying stale edits over a week of newer remote history does more
			// harm than dropping them.
			if (
				commandSync.createdAt.getTime() <
				Date.now() - OFFLINE_SYNC_COMMAND_MAX_AGE_MS
			) {
				console.warn(
					"Dropping expired offline sync command:",
					commandSync.commandId,
				);
				await this.backend.clearOfflineSyncCommands(
					commandSync.commandId,
					appId,
					boardId,
				);
				continue;
			}

			try {
				for (const chunk of chunkCommandsForSync(commandSync.commands)) {
					await fetcher(
						this.backend.profile!,
						`apps/${appId}/board/${boardId}`,
						{
							method: "POST",
							body: JSON.stringify({
								commands: chunk,
							}),
						},
						this.backend.auth,
					);
				}
				await this.backend.clearOfflineSyncCommands(
					commandSync.commandId,
					appId,
					boardId,
				);
				pushedBatches += 1;
				console.log("Executed offline sync command:", commandSync.commandId);
			} catch (e) {
				// Keep the batch queued and stop: later batches must not overtake it.
				console.warn(
					"Failed to push offline sync command; keeping it queued:",
					e,
				);
				failed = true;
				break;
			}
		}

		if (!failed && pushedBatches > 0) {
			await this.recordLineageAfterPush(appId, boardId);
		}

		return { failed, pushedBatches };
	}

	async retryOfflineSync(
		appId: string,
		boardId: string,
	): Promise<{ pushedBatches: number; remainingBatches: number }> {
		const countRemaining = async () =>
			(await this.backend.getOfflineSyncCommands(appId, boardId)).length;

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) {
			toast.error("Cannot sync queued edits while offline or signed out.");
			return { pushedBatches: 0, remainingBatches: await countRemaining() };
		}

		const { failed, pushedBatches } = await this.drainOfflineSyncQueue(
			appId,
			boardId,
		);
		const remainingBatches = await countRemaining();

		if (failed) {
			toast.error(
				`Sync retry failed — ${remainingBatches} edit ${remainingBatches === 1 ? "batch is" : "batches are"} still queued.`,
			);
		} else if (pushedBatches > 0) {
			toast.success("Queued edits synced to the server.");
		} else {
			toast.info("No queued edits to sync.");
		}

		return { pushedBatches, remainingBatches };
	}

	/**
	 * Push executed commands to the server in order-preserving, size-bounded chunks.
	 *
	 * Every failure path appends the undelivered tail to the offline sync queue, and a
	 * non-empty queue forces queueing instead of a direct push: a later small command must
	 * never overtake an earlier failed batch, otherwise the remote board becomes "newer"
	 * while missing that batch and the next sync clobbers the local content with it.
	 */
	private async syncExecutedCommandsToServer(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<void> {
		if (commands.length === 0) return;

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) return;

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			await this.backend.pushOfflineSyncCommand(appId, boardId, commands);
			return;
		}

		const pending = await this.backend.getOfflineSyncCommands(appId, boardId);
		if (pending.length > 0) {
			await this.backend.pushOfflineSyncCommand(appId, boardId, commands);
			this.notifyEditsQueued(appId, boardId);
			return;
		}

		const chunks = chunkCommandsForSync(commands);
		for (let index = 0; index < chunks.length; index++) {
			try {
				await fetcher(
					this.backend.profile,
					`apps/${appId}/board/${boardId}`,
					{
						method: "POST",
						body: JSON.stringify({
							commands: chunks[index],
						}),
					},
					this.backend.auth,
				);
			} catch (error) {
				console.error(
					"Failed to push commands to server; queueing the remainder for ordered sync:",
					error,
				);
				await this.backend.pushOfflineSyncCommand(
					appId,
					boardId,
					chunks.slice(index).flat(),
				);
				this.notifyEditsQueued(appId, boardId);
				return;
			}
		}

		await this.recordLineageAfterPush(appId, boardId);
	}

	async executeCommand(
		appId: string,
		boardId: string,
		command: IGenericCommand,
	): Promise<IGenericCommand> {
		const executedCommand = await invoke<IGenericCommand>("execute_command", {
			appId: appId,
			boardId: boardId,
			command: command,
		});

		await this.syncExecutedCommandsToServer(appId, boardId, [executedCommand]);

		return executedCommand;
	}

	async executeCommands(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<IGenericCommand[]> {
		const executedCommands = await invoke<IGenericCommand[]>(
			"execute_commands",
			{
				appId: appId,
				boardId: boardId,
				commands: commands,
			},
		);

		await this.syncExecutedCommandsToServer(appId, boardId, executedCommands);

		return executedCommands;
	}

	async applyFlowScript(
		appId: string,
		boardId: string,
		flowscript: string,
		currentLayer?: string,
		catalogNodes?: INode[],
		allowDeletions = false,
	): Promise<IApplyFlowScriptResponse> {
		const result = await invoke<IApplyFlowScriptResponse>("apply_flowscript", {
			appId,
			boardId,
			flowscript,
			currentLayer,
			catalogNodes: getAppPackageCatalogNodes(catalogNodes),
			allowDeletions,
		});

		if (result.commands.length === 0) {
			return result;
		}

		await this.syncExecutedCommandsToServer(appId, boardId, result.commands);

		return result;
	}

	async getFlowScript(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		anchors = true,
	): Promise<string> {
		try {
			return await invoke<string>("get_flowscript", {
				appId,
				boardId,
				version,
				anchors,
			});
		} catch {
			const isOffline = await this.backend.isOffline(appId);
			if (isOffline || !this.backend.profile || !this.backend.auth) {
				throw new Error(`Board not found: ${boardId}`);
			}
			const params = new URLSearchParams();
			if (version) params.set("version", version.join("_"));
			params.set("anchors", String(anchors));
			const response = await fetcher<{ flowscript: string }>(
				this.backend.profile,
				`apps/${appId}/board/${boardId}/flowscript?${params}`,
				{ method: "GET" },
				this.backend.auth,
			);
			return response.flowscript;
		}
	}

	async lintFlowScript(flowscript: string): Promise<IFlowScriptDiagnostic[]> {
		return await invoke<IFlowScriptDiagnostic[]>("lint_flowscript", {
			flowscript,
		});
	}

	async getExecutionElements(
		appId: string,
		boardId: string,
		pageId: string,
		wildcard = false,
		version?: [number, number, number],
	): Promise<Record<string, unknown>> {
		// Try local execution first
		const localElements = await invoke<Record<string, unknown>>(
			"get_execution_elements",
			{
				appId,
				boardId,
				pageId,
				wildcard,
				version,
			},
		);

		console.log("[BoardState] getExecutionElements local result:", {
			boardId,
			pageId,
			wildcard,
			localElementKeys: Object.keys(localElements),
		});

		// For offline apps or if we have local elements, return them
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || Object.keys(localElements).length > 0) {
			return localElements;
		}

		// Try remote API if online and no local elements
		if (this.backend.profile && this.backend.auth) {
			try {
				const params = new URLSearchParams();
				params.set("page_id", pageId);
				if (wildcard) params.set("wildcard", "true");
				if (version) params.set("version", version.join("_"));

				const response = await fetcher<{ elements: Record<string, unknown> }>(
					this.backend.profile,
					`apps/${appId}/board/${boardId}/elements?${params.toString()}`,
					{ method: "GET" },
					this.backend.auth,
				);
				console.log(
					"[BoardState] getExecutionElements remote fallback result:",
					{
						boardId,
						pageId,
						wildcard,
						remoteElementKeys: Object.keys(response.elements ?? {}),
					},
				);
				return response.elements;
			} catch (error) {
				console.warn("Failed to fetch execution elements from API:", error);
			}
		}

		return localElements;
	}

	async copilot_chat(
		scope: CopilotScope,
		board: IBoard | null,
		catalogNodes: INode[] | undefined,
		selectedNodeIds: string[],
		currentSurface: SurfaceComponent[] | null,
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
		nested?: boolean,
		readOnly?: boolean,
		toolContext?: CopilotToolContext,
		requestId?: string,
		rawUserPrompt?: string,
	): Promise<UnifiedCopilotResponse> {
		flowPilotDebugLog(
			"[copilot_chat] Calling with scope:",
			scope,
			"runContext:",
			runContext,
		);

		const channel = new Channel<string>();
		if (onToken) {
			channel.onmessage = onToken;
		}

		const actualToken = token ?? this.backend.auth?.user?.access_token;
		const appPackageCatalogNodes = getAppPackageCatalogNodes(catalogNodes);

		return await invoke("copilot_chat", {
			scope,
			board,
			catalogNodes: appPackageCatalogNodes,
			selectedNodeIds,
			currentSurface,
			selectedComponentIds,
			userPrompt,
			history,
			currentImages: requestImages,
			modelId,
			reasoningEffort,
			channel,
			token: actualToken,
			runContext,
			actionContext,
			nested,
			readOnly,
			toolContext,
			requestId,
			rawUserPrompt,
		});
	}

	async cancelCopilotChat(requestId: string): Promise<void> {
		await invoke<boolean>("cancel_copilot_chat", { requestId });
	}

	async flowIrCommitDisposition(
		token: FlowIrCommitToken,
		disposition: FlowIrCommitDisposition,
	): Promise<FlowIrCommitDispositionResult> {
		return await invoke<FlowIrCommitDispositionResult>(
			"flowpilot_flow_ir_commit_disposition",
			{ token, disposition },
		);
	}

	async applyFlowIrCommit(
		appId: string,
		token: FlowIrCommitToken,
	): Promise<IApplyFlowIrCommitResponse> {
		const result = await invoke<IApplyFlowIrCommitResponse>(
			"flowpilot_apply_flow_ir_commit",
			{
				appId,
				token,
			},
		);
		if (result.status !== "applied" || result.commands.length === 0) {
			return result;
		}

		try {
			await this.syncExecutedCommandsToServer(
				appId,
				token.board_id,
				result.commands,
			);
		} catch (error) {
			// Native apply has already committed and acknowledged the exact batch. A
			// remote-sync bookkeeping failure is recoverable and must not make the caller
			// retry or dismiss a commit that is already present locally.
			const warning = `Typed workflow applied locally; remote synchronization must retry: ${getErrorMessage(error, "Unknown sync error")}`;
			console.error(warning, error);
			return {
				...result,
				diagnostics: [...result.diagnostics, warning],
			};
		}
		return result;
	}

	async prerunBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IPrerunBoardResponse> {
		const isOffline = await this.backend.isOffline(appId);

		// Helper to build prerun response from local board
		const buildLocalPrerun = async (): Promise<IPrerunBoardResponse> => {
			const board: IBoard = await invoke("get_board", {
				appId,
				boardId,
				version,
			});

			const runtimeVariables = Object.values(board.variables)
				.filter((v) => v.runtime_configured)
				.map((v) => ({
					id: v.id,
					name: v.name,
					description: v.description ?? undefined,
					data_type: v.data_type,
					value_type: v.value_type,
					secret: v.secret,
					schema: v.schema ?? undefined,
				}));

			const {
				oauth_requirements,
				requires_local_execution,
				execution_mode,
				can_execute_locally,
			} = extractOAuthRequirementsFromBoard(board);

			// Collect all WASM (external) node package_ids and permissions
			const wasmPackageIds = new Set<string>();
			const wasmPackagePermissions: Record<string, string[]> = {};
			const collectWasm = (node: INode) => {
				if (node.wasm?.package_id) {
					wasmPackageIds.add(node.wasm.package_id);
					if (node.wasm.permissions?.length) {
						const existing = wasmPackagePermissions[node.wasm.package_id] ?? [];
						for (const perm of node.wasm.permissions) {
							if (!existing.includes(perm)) existing.push(perm);
						}
						wasmPackagePermissions[node.wasm.package_id] = existing;
					}
				}
			};
			for (const node of Object.values(board.nodes)) collectWasm(node);
			for (const layer of Object.values(board.layers)) {
				for (const node of Object.values(layer.nodes)) collectWasm(node);
			}

			return {
				runtime_variables: runtimeVariables,
				oauth_requirements,
				requires_local_execution,
				execution_mode,
				can_execute_locally,
				has_wasm_nodes: wasmPackageIds.size > 0,
				wasm_package_ids: Array.from(wasmPackageIds),
				wasm_package_permissions: wasmPackagePermissions,
			};
		};

		// Offline apps: always use local board data
		if (isOffline) {
			return buildLocalPrerun();
		}

		return resolveLocalFirstPrerun({
			label: "prerunBoard",
			buildLocal: buildLocalPrerun,
			fetchRemote:
				this.backend.profile && this.backend.auth
					? async () => {
							let url = `apps/${appId}/board/${boardId}/prerun`;
							if (version) {
								url += `?version=${version.join("_")}`;
							}

							return fetcher<IPrerunBoardResponse>(
								this.backend.profile!,
								url,
								{ method: "GET" },
								this.backend.auth!,
							);
						}
					: undefined,
		});
	}
}
