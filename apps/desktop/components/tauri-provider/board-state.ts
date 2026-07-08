import { Channel, invoke } from "@tauri-apps/api/core";
import {
	type ChatImage,
	type CopilotScope,
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
import { resolveLocalFirstPrerun } from "./prerun-utils";

interface DiffEntry {
	path: string;
	local: any;
	remote: any;
}

interface SystemTimeLike {
	secs_since_epoch?: number;
	nanos_since_epoch?: number;
}

const REMOTE_BOARD_APPLIED_EVENT = "flow:remote-board-applied";

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

const logBoardDifferences = (localBoard: IBoard, remoteBoard: IBoard) => {
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
const preserveSecretValues = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): IBoard => {
	if (!localBoard) return remoteBoard;

	for (const [varId, remoteVar] of Object.entries(remoteBoard.variables)) {
		const localVar = localBoard.variables[varId];
		if (
			localVar?.secret &&
			remoteVar.secret &&
			remoteVar.default_value == null &&
			localVar.default_value != null
		) {
			remoteVar.default_value = localVar.default_value;
		}
	}

	return remoteBoard;
};

const comparableNodeWithoutRuntimeHash = (
	node: INode,
	localNode?: INode,
): INode => {
	const comparable = structuredClone(node);
	comparable.hash = undefined;

	if (comparable.wasm == null && localNode?.wasm != null) {
		comparable.wasm = structuredClone(localNode.wasm);
	}

	return comparable;
};

const preserveNodeRuntimeFields = (
	remoteNode: INode,
	localNode?: INode,
): INode => {
	if (!localNode) return remoteNode;

	const nodesMatchIgnoringRuntimeHash = isEqual(
		comparableNodeWithoutRuntimeHash(remoteNode, localNode),
		comparableNodeWithoutRuntimeHash(localNode),
	);

	if (
		localNode.hash != null &&
		(remoteNode.hash == null || nodesMatchIgnoringRuntimeHash)
	) {
		remoteNode.hash = localNode.hash;
	}

	if (remoteNode.wasm == null && localNode.wasm != null) {
		remoteNode.wasm = structuredClone(localNode.wasm);
	}

	return remoteNode;
};

const preserveBoardRuntimeFields = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): IBoard => {
	if (!localBoard) return remoteBoard;

	for (const [nodeId, remoteNode] of Object.entries(remoteBoard.nodes)) {
		preserveNodeRuntimeFields(remoteNode, localBoard.nodes[nodeId]);
	}

	for (const [layerId, remoteLayer] of Object.entries(remoteBoard.layers)) {
		const localLayer = localBoard.layers[layerId];
		if (!localLayer) continue;

		for (const [nodeId, remoteNode] of Object.entries(remoteLayer.nodes)) {
			preserveNodeRuntimeFields(remoteNode, localLayer.nodes[nodeId]);
		}
	}

	return remoteBoard;
};

const getAppPackageCatalogNodes = (
	catalogNodes: INode[] | undefined,
): INode[] | undefined => {
	const packageNodes = catalogNodes?.filter((node) =>
		Boolean(node.wasm?.package_id),
	);

	return packageNodes?.length ? packageNodes : undefined;
};

const cloneBoard = (board: IBoard): IBoard => structuredClone(board);

const systemTimeToNumber = (time?: SystemTimeLike): number => {
	if (!time) return 0;
	return (
		(time.secs_since_epoch ?? 0) * 1_000_000_000 + (time.nanos_since_epoch ?? 0)
	);
};

const hasIncompletePageIds = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): boolean =>
	(remoteBoard.page_ids?.length ?? 0) === 0 &&
	(localBoard?.page_ids?.length ?? 0) > 0;

const shouldApplyRemoteBoard = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): boolean => {
	if (!localBoard) return true;

	if (hasIncompletePageIds(remoteBoard, localBoard)) {
		return false;
	}

	const remoteUpdated = systemTimeToNumber(remoteBoard.updated_at);
	const localUpdated = systemTimeToNumber(localBoard.updated_at);

	if (remoteUpdated > 0 && localUpdated > 0 && remoteUpdated < localUpdated) {
		return false;
	}

	return true;
};

const mergeRemoteBoard = (remoteBoard: IBoard, localBoard?: IBoard): IBoard => {
	const merged = preserveBoardRuntimeFields(
		preserveSecretValues(cloneBoard(remoteBoard), localBoard),
		localBoard,
	);

	if (hasIncompletePageIds(merged, localBoard)) {
		merged.page_ids = localBoard?.page_ids ?? merged.page_ids;
	}

	return merged;
};

const boardsDifferIgnoringUpdatedAt = (
	incomingBoard: IBoard,
	currentBoard?: IBoard,
): boolean => {
	if (!currentBoard) return true;

	const comparableBoard = cloneBoard(incomingBoard);
	comparableBoard.updated_at = currentBoard.updated_at;

	return !isEqual(comparableBoard, currentBoard);
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

const getRemoteBoardSkipReason = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): string | null => {
	if (!localBoard) return null;

	if (hasIncompletePageIds(remoteBoard, localBoard)) {
		return "remote page_ids empty while local board still has pages";
	}

	const remoteUpdated = systemTimeToNumber(remoteBoard.updated_at);
	const localUpdated = systemTimeToNumber(localBoard.updated_at);

	if (remoteUpdated > 0 && localUpdated > 0 && remoteUpdated < localUpdated) {
		return "remote board updated_at is older than local board";
	}

	return null;
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
					const skipReason = getRemoteBoardSkipReason(board, localBoard);
					const nextBoard = shouldApplyRemoteBoard(board, localBoard)
						? mergeRemoteBoard(board, localBoard)
						: (localBoard ?? mergeRemoteBoard(board, localBoard));

					if (localBoard && nextBoard === localBoard) {
						console.warn(
							"Skipping stale or incomplete remote board during board list sync:",
							{
								boardId: board.id,
								skipReason,
								localPageIds: localBoard.page_ids,
								remotePageIds: board.page_ids,
								localUpdatedAt: localBoard.updated_at,
								remoteUpdatedAt: board.updated_at,
							},
						);
					}

					if (boardsDifferIgnoringUpdatedAt(nextBoard, localBoard)) {
						console.log("Board data changed, updating local state:");
						await invoke("upsert_board", {
							appId: appId,
							boardId: nextBoard.id,
							name: nextBoard.name,
							description: nextBoard.description,
							logLevel: nextBoard.log_level,
							stage: nextBoard.stage,
							executionMode: nextBoard.execution_mode,
							boardData: nextBoard,
						});
					}

					mergedBoards.set(board.id, nextBoard);
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
				const url = `apps/${appId}/board/${boardId}`;
				const remoteData = await fetcher<IBoard>(
					this.backend.profile,
					url,
					{ method: "GET" },
					this.backend.auth,
				);

				if (remoteData) {
					const merged = mergeRemoteBoard(remoteData, board);
					if (
						boardsDifferIgnoringUpdatedAt(merged, board) &&
						typeof version === "undefined"
					) {
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
					}
					return merged;
				}
			} catch (e) {
				console.warn(
					"[BoardState] forceFresh sync failed, using local board:",
					e,
				);
			}
			return board;
		}

		const getOfflineSyncCommands =
			this.backend.getOfflineSyncCommands.bind(this);
		const clearOfflineSyncCommands =
			this.backend.clearOfflineSyncCommands.bind(this);

		const promise = injectDataFunction(
			async () => {
				const unsyncedCommands = await getOfflineSyncCommands(appId, boardId);
				for (const commandSync of unsyncedCommands) {
					try {
						// Only sync commands up to a week old
						if (
							commandSync.createdAt.getTime() <
							Date.now() - 7 * 24 * 60 * 60 * 1000
						)
							await fetcher(
								this.backend.profile!,
								`apps/${appId}/board/${boardId}`,
								{
									method: "POST",
									body: JSON.stringify({
										commands: commandSync.commands,
									}),
								},
								this.backend.auth,
							);
						console.log(
							"Executed offline sync command:",
							commandSync.commandId,
						);
						await clearOfflineSyncCommands(
							commandSync.commandId,
							appId,
							boardId,
						);
					} catch (e) {
						console.warn("Failed to execute offline sync command:", e);
					}
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

				const shouldUseRemote = shouldApplyRemoteBoard(remoteData, board);
				const skipReason = getRemoteBoardSkipReason(remoteData, board);
				const merged = shouldUseRemote
					? mergeRemoteBoard(remoteData, board)
					: board;

				if (!shouldUseRemote) {
					console.warn(
						"Skipping stale or incomplete remote board during board sync:",
						{
							boardId,
							skipReason,
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
					boardsDifferIgnoringUpdatedAt(merged, board) &&
					typeof version === "undefined"
				) {
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
					dispatchRemoteBoardApplied(appId, boardId);
				} else {
					console.log("Board data is up to date, no update needed.");
				}

				return merged;
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
		const board = await this.getBoard(appId, boardId, undefined, true);
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

		await fetcher(
			this.backend.profile,
			`apps/${appId}/board/${boardId}/undo`,
			{
				method: "PATCH",
				body: JSON.stringify({
					commands: commands,
				}),
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

		await fetcher(
			this.backend.profile,
			`apps/${appId}/board/${boardId}/redo`,
			{
				method: "PATCH",
				body: JSON.stringify({
					commands: commands,
				}),
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

		const boardUpdate = await fetcher<{ id: string }>(
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
	}

	async closeBoard(boardId: string) {
		await invoke("close_board", {
			boardId: boardId,
		});
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

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) {
			return executedCommand;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			await this.backend.pushOfflineSyncCommand(appId, boardId, [
				executedCommand,
			]);
			return executedCommand;
		}

		try {
			await fetcher(
				this.backend.profile,
				`apps/${appId}/board/${boardId}`,
				{
					method: "POST",
					body: JSON.stringify({
						commands: [executedCommand],
					}),
				},
				this.backend.auth,
			);
		} catch (error) {
			console.error("Failed to push command to server:", error);
			await this.backend.pushOfflineSyncCommand(appId, boardId, [
				executedCommand,
			]);
		}

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

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) {
			return executedCommands;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			await this.backend.pushOfflineSyncCommand(
				appId,
				boardId,
				executedCommands,
			);
			return executedCommands;
		}

		try {
			await fetcher(
				this.backend.profile,
				`apps/${appId}/board/${boardId}`,
				{
					method: "POST",
					body: JSON.stringify({
						commands: executedCommands,
					}),
				},
				this.backend.auth,
			);
		} catch (error) {
			console.error("Failed to push commands to server:", error);
			await this.backend.pushOfflineSyncCommand(
				appId,
				boardId,
				executedCommands,
			);
		}

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

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) {
			return result;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			await this.backend.pushOfflineSyncCommand(
				appId,
				boardId,
				result.commands,
			);
			return result;
		}

		try {
			await fetcher(
				this.backend.profile,
				`apps/${appId}/board/${boardId}`,
				{
					method: "POST",
					body: JSON.stringify({
						commands: result.commands,
					}),
				},
				this.backend.auth,
			);
		} catch (error) {
			console.error("Failed to push FlowScript commands to server:", error);
			await this.backend.pushOfflineSyncCommand(
				appId,
				boardId,
				result.commands,
			);
		}

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
	): Promise<Record<string, unknown>> {
		// Try local execution first
		const localElements = await invoke<Record<string, unknown>>(
			"get_execution_elements",
			{
				boardId,
				pageId,
				wildcard,
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
		token?: string,
		runContext?: IRunContext,
		actionContext?: UIActionContext,
		nested?: boolean,
		readOnly?: boolean,
	): Promise<UnifiedCopilotResponse> {
		console.log(
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
			channel,
			token: actualToken,
			runContext,
			actionContext,
			nested,
			readOnly,
		});
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
