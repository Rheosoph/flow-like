import {
	type BoardEditJob,
	type BoardEditJobDeliveryClaim,
	type BoardEditJobResolution,
	type ChatImage,
	type CopilotScope,
	type CopilotToolContext,
	type FlowIrCommitDisposition,
	type FlowIrCommitDispositionResult,
	type FlowIrCommitToken,
	IAppVisibility,
	type IApplyFlowIrCommitResponse,
	type IApplyFlowScriptResponse,
	type IBoard,
	type IBoardMutationOptions,
	type IBoardServerResetResult,
	type IBoardState,
	type IBoardSummary,
	type IBoardSummaryInclude,
	type IBoardSyncStatus,
	type IBoardVariables,
	type ICheckFlowScriptReconcileResponse,
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
	type IScopedFlowScriptResponse,
	type ISettingsProfile,
	type IVersionType,
	type ProgressToastData,
	type UIActionContext,
	type UnifiedChatMessage,
	type UnifiedCopilotResponse,
	checkOAuthTokens,
	dispatchBoardDelivered,
	dispatchBoardSyncChanged,
	dispatchBoardSyncRecoveryRequest,
	extractOAuthRequirementsFromBoard,
	finishAllProgressToasts,
	injectDataFunction,
	isEqual,
	showProgressToast,
} from "@flow-like/flow-like-ui";
import type { IJwks, IRealtimeAccess } from "@flow-like/flow-like-ui";
import type { IProfile } from "@flow-like/flow-like-ui";
import type {
	CanvasSettings,
	SurfaceComponent,
} from "@flow-like/flow-like-ui/components/a2ui/types";
import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import {
	BoardSyncClient,
	type IBoardSyncRequest,
	type IBoardSyncResponse,
} from "@flow-like/flow-like-ui/lib/board-sync";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import { flowPilotDebugLog } from "@flow-like/flow-like-ui/lib/flowpilot-debug";
import { flowIrCommitDeliveryId } from "@flow-like/flow-like-ui/lib/flowpilot/board-edit-job-delivery";
import {
	FLOWSCRIPT_APPLY_FAILURE_PATH,
	type FlowScriptApplyOrigin,
	type FlowScriptApplyOutcome,
	type IFlowScriptApplyFailureReport,
	flowScriptApplyOutcome,
} from "@flow-like/flow-like-ui/lib/flowscript-apply-failure";
import { normalizeBoardVersion } from "@flow-like/flow-like-ui/lib/schema/flow/board-version";
import { createId } from "@paralleldrive/cuid2";
import { getVersion } from "@tauri-apps/api/app";
import { Channel, invoke } from "@tauri-apps/api/core";
import { confirm } from "@tauri-apps/plugin-dialog";
import { isObject } from "lodash-es";
import type { CSSProperties } from "react";
import { toast } from "sonner";
import { fetcher, streamFetcher } from "../../lib/api";
import {
	dispatchFlowNotificationEvent,
	dispatchFlowNotificationEvents,
} from "../../lib/flow-notification-events";
import { oauthConsentStore, oauthTokenStore } from "../../lib/oauth-db";
import { oauthService } from "../../lib/oauth-service";
import { desktopPlatform } from "../../lib/platform";
import {
	ensureRpaSystemPermissions,
	requestRpaAutomationConsent,
} from "../rpa";
import type { TauriBackend } from "../tauri-provider";
import {
	getRemoteBoardSkipReason,
	shouldApplyRemoteBoard,
} from "./board-merge";
import { mergeBoardOffThread } from "./board-sync";
import {
	CommandSyncPayloadTooLargeError,
	type CommandSyncRemoteIdentity,
	OFFLINE_SYNC_COMMAND_MAX_AGE_MS,
	chunkCommandsForSync,
	chunkLegacyCommandsForRecovery,
	commandSyncHasPendingMutation,
	evaluateBoardLineage,
	evaluateCommandSyncRemoteIdentity,
	findUnresolvedPinReferences,
	repairUnreplayableCommandBatch,
	selectDiscardableSyncRows,
	summarizeBoardSyncQueue,
	systemTimeToNanos,
} from "./command-sync";
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

interface OfflineSyncFailure {
	status?: number;
	message: string;
}

interface OfflineSyncDrainResult {
	failed: boolean;
	pushedBatches: number;
	failure?: OfflineSyncFailure;
}

/**
 * A server-authoritative reset was requested while undelivered edits are still queued.
 *
 * Carries the queue it refused to destroy so the caller can itemize it before asking again with
 * `discardQueuedEdits`.
 */
export class BoardSyncDiscardRequiredError extends Error {
	constructor(readonly status: IBoardSyncStatus) {
		super(
			`Fetching the server board would discard ${status.pendingBatches} queued edit ${
				status.pendingBatches === 1 ? "batch" : "batches"
			} that the server has not accepted.`,
		);
		this.name = "BoardSyncDiscardRequiredError";
	}
}

/** Where materializing a board this device does not have gave up. */
export type BoardMaterializationPhase =
	| "gated"
	| "fetch"
	| "persist"
	| "verify";

/**
 * A board the device does not hold could not be brought onto disk.
 *
 * Every caller of `getBoard` treats a resolved promise as "the board is readable locally" — the
 * page lookup and the pre-run gate both re-read from disk immediately afterwards. Reporting the
 * download as a success while the write failed makes every retry re-run an identical, invisible
 * failure, so the local-miss path fails loudly instead, naming the step that broke.
 */
export class BoardMaterializationError extends Error {
	constructor(
		readonly boardId: string,
		readonly phase: BoardMaterializationPhase,
		options?: { cause?: unknown },
	) {
		super(
			`Board ${boardId} could not be made available on this device (${phase})`,
			options,
		);
		this.name = "BoardMaterializationError";
	}
}

/**
 * Command-batch rejections carry an index, an entity id and a rollback trace, which overflow a
 * toast. The full text stays on the queued row and is shown in the recovery dialog.
 */
const MAX_TOAST_FAILURE_CHARS = 160;

/**
 * A stalled queue is only actionable if the user can tell an outage apart from a payload the
 * server will never accept. Surface the transport's own status and message rather than a generic
 * "sync incomplete".
 */
function describeOfflineSyncFailure(failure?: OfflineSyncFailure): string {
	if (!failure) return "The server did not accept the queued edits.";
	const detail =
		failure.message.length > MAX_TOAST_FAILURE_CHARS
			? `${failure.message.slice(0, MAX_TOAST_FAILURE_CHARS).trimEnd()}…`
			: failure.message;
	if (failure.status === undefined) return detail;
	return `HTTP ${failure.status}: ${detail}`;
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
	private readonly offlineSyncDrains = new Map<
		string,
		Promise<{ failed: boolean; pushedBatches: number }>
	>();
	private readonly boardMutationSequences = new Map<string, Promise<void>>();
	/** Remote boards are fetched incrementally; this holds the last assembled copy per board. */
	private readonly remoteBoardSync = new BoardSyncClient();
	/**
	 * Local boards cross the Tauri bridge incrementally too: `get_board` serialises the whole
	 * board on every refetch, and refetches happen after every own or peer edit. Kept separate
	 * from `remoteBoardSync` because the two sides hold different revisions of the same board.
	 */
	private readonly localBoardSync = new BoardSyncClient();

	/** The local board, transferred as a diff against the last one this client assembled. */
	private fetchLocalBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IBoard> {
		return this.localBoardSync.sync(
			appId,
			boardId,
			version,
			(request: IBoardSyncRequest) =>
				invoke<IBoardSyncResponse>("sync_board", {
					appId,
					boardId,
					version,
					request,
				}),
		);
	}

	/**
	 * The server's current board, transferred as a diff against the last one this client
	 * assembled. Semantically identical to a full GET, including for `resetBoardFromServer`.
	 */
	private fetchRemoteBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		profile: IProfile | undefined = this.backend.profile,
		auth = this.backend.auth,
	): Promise<IBoard> {
		if (!profile) throw new Error("No profile set for remote board fetch");
		const params = version ? `?version=${version.join("_")}` : "";
		return this.remoteBoardSync.sync(
			appId,
			boardId,
			version,
			(request: IBoardSyncRequest) =>
				fetcher<IBoardSyncResponse>(
					profile,
					`apps/${appId}/board/${boardId}/sync${params}`,
					{ method: "POST", body: JSON.stringify(request) },
					auth,
				),
		);
	}

	constructor(private readonly backend: TauriBackend) {}

	private async remoteBoardDeliveryIdentity(
		appId: string,
		boardId?: string,
	): Promise<CommandSyncRemoteIdentity | undefined> {
		// Read native app metadata at the mutation boundary. The IndexedDB visibility cache is
		// useful for rendering, but a stale value must not decide whether an edit targets Hub.
		const app = await invoke<{ visibility?: IAppVisibility }>("get_app", {
			appId,
		});
		if ((app.visibility ?? IAppVisibility.Offline) === IAppVisibility.Offline) {
			if (boardId) {
				const pending = (
					await this.backend.getOfflineSyncCommands(appId, boardId)
				).filter(commandSyncHasPendingMutation);
				if (pending.length > 0) {
					throw new Error(
						"This board still has queued remote edits. Retry or recover them before making local-only edits that would fork their ordered state.",
					);
				}
			}
			return undefined;
		}
		const principal = this.backend.auth?.user?.profile.sub;
		if (!this.backend.profile || !principal) {
			throw new Error(
				"A remote board edit cannot be applied while its authenticated account and Hub destination are unavailable. Sign in, then retry the edit.",
			);
		}
		const identity: CommandSyncRemoteIdentity = {
			remoteIdentityVersion: 1,
			remoteProfileId: this.backend.profile.id,
			remotePrincipalId: principal,
			remoteHub: this.backend.profile.hub,
		};
		if (boardId) {
			const pending = (
				await this.backend.getOfflineSyncCommands(appId, boardId)
			).filter(commandSyncHasPendingMutation);
			for (const entry of pending) {
				if (entry.remoteIdentityVersion !== 1) {
					throw new Error(
						"This board has an older queued edit with no provable account/Hub owner. Use Retry queued edits to review and bind it before making another edit.",
					);
				}
				const owner = evaluateCommandSyncRemoteIdentity(entry, identity);
				if (!owner.apply) {
					throw new Error(
						`This board has an earlier queued edit for another remote identity: ${owner.refusalReason}. Switch back to that account/Hub and retry it before making another edit.`,
					);
				}
				if (
					entry.blockedReason ||
					entry.createdAt.getTime() <
						Date.now() - OFFLINE_SYNC_COMMAND_MAX_AGE_MS
				) {
					throw new Error(
						entry.blockedReason ??
							"This board has an expired queued edit that requires recovery before another dependent edit can be applied.",
					);
				}
			}
		}
		return identity;
	}

	/**
	 * Preserve native and remote mutation order for one board. The native board lock protects the
	 * local graph, while this renderer-side tail keeps its durable outbox handoff in the same order.
	 */
	private async sequenceBoardMutation<T>(
		appId: string,
		boardId: string,
		operation: () => Promise<T>,
	): Promise<T> {
		const key = `${appId}\u001f${boardId}`;
		const previous = this.boardMutationSequences.get(key) ?? Promise.resolve();
		const result = previous.catch(() => undefined).then(operation);
		const tail = result.then(
			() => undefined,
			() => undefined,
		);
		this.boardMutationSequences.set(key, tail);
		try {
			return await result;
		} finally {
			if (this.boardMutationSequences.get(key) === tail) {
				this.boardMutationSequences.delete(key);
			}
		}
	}

	private async assertNoPendingNativeBoardDelivery(
		appId: string,
		boardId: string,
	): Promise<void> {
		const jobs = await this.listBoardEditJobs(appId, boardId, false);
		const pending = jobs.find(
			(job) =>
				job.phase === "applying" ||
				job.phase === "applied_pending_delivery" ||
				job.phase === "failed",
		);
		if (pending) {
			throw new Error(
				`Board edits are paused while FlowPilot finishes durable delivery of review ${pending.jobId}. Reopen this board and let receipt recovery complete.`,
			);
		}
	}

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
						const pendingMutations = pendingSync.filter(
							commandSyncHasPendingMutation,
						);
						if (pendingMutations.length > 0) {
							// Local edits are still queued for the server; the remote snapshot
							// predates them and applying it would clobber the local content.
							console.warn(
								"Skipping remote board with pending offline sync commands:",
								{
									boardId: board.id,
									pendingBatches: pendingMutations.length,
								},
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

	async getBoardSummaries(
		appId: string,
		include?: IBoardSummaryInclude[],
	): Promise<IBoardSummary[]> {
		const withNodeTypes = include?.includes("node_types") === true;
		const withMetrics = include?.includes("metrics") === true;
		const local: IBoardSummary[] = await invoke("get_app_board_summaries", {
			appId,
			withNodeTypes,
			withMetrics,
		});
		const byId = new Map(local.map((summary) => [summary.id, summary]));

		// This listing is the app-level safety net that materializes boards the device never
		// downloaded, so an app whose visibility is merely unknown must not gate it shut.
		const localOnly = await this.backend.isLocalOnly(appId);
		if (localOnly || !this.backend.profile || !this.backend.auth) {
			return Array.from(byId.values());
		}

		const query = include?.length ? `?include=${include.join(",")}` : "";
		let remote: IBoardSummary[];
		try {
			remote = await fetcher<IBoardSummary[]>(
				this.backend.profile,
				`apps/${appId}/board/summaries${query}`,
				{ method: "GET" },
				this.backend.auth,
			);
		} catch (error) {
			console.warn(
				"[BoardState] remote board summaries unavailable, using local:",
				error,
			);
			return Array.from(byId.values());
		}

		// Boards the server has that this device does not, or that changed remotely, are pulled
		// through `getBoard` — local-first and incremental — instead of the old full-list download.
		const stale: string[] = [];
		for (const summary of remote) {
			const localSummary = byId.get(summary.id);
			const remoteNanos = systemTimeToNanos(summary.updatedAt);
			const localNanos = systemTimeToNanos(localSummary?.updatedAt);
			if (!localSummary || (remoteNanos > 0 && remoteNanos > localNanos)) {
				stale.push(summary.id);
			}
			// Remote metadata is authoritative for the listing; local wins only when the local
			// copy is the newer one (queued offline edits).
			byId.set(
				summary.id,
				localSummary && localNanos > remoteNanos
					? { ...summary, ...localSummary, pages: summary.pages }
					: summary,
			);
		}
		if (stale.length > 0 && this.backend.queryClient) {
			this.backend.backgroundTaskHandler(
				Promise.allSettled(
					stale.map((boardId) => this.getBoard(appId, boardId)),
				).then(() => undefined),
			);
		}

		return Array.from(byId.values());
	}

	async getBoardVariables(appId: string): Promise<IBoardVariables[]> {
		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline && this.backend.profile && this.backend.auth) {
			try {
				return await fetcher<IBoardVariables[]>(
					this.backend.profile,
					`apps/${appId}/board/variables`,
					{ method: "GET" },
					this.backend.auth,
				);
			} catch (error) {
				console.warn(
					"[BoardState] remote board variables unavailable, using local:",
					error,
				);
			}
		}
		return invoke("get_app_board_variables", { appId });
	}

	async getCatalog(appId: string): Promise<INode[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline && this.backend.profile && this.backend.auth) {
			try {
				const nodes = await fetcher<INode[]>(
					this.backend.profile,
					`apps/${appId}/nodes`,
					{ method: "GET" },
					this.backend.auth,
				);
				// The remote catalog is what the server compares against when it decides which
				// nodes may ship lean, so only it may feed remote-board hydration.
				this.remoteBoardSync.setCatalog(appId, nodes);
				return nodes;
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
	/**
	 * Bring a board this device does not have onto disk, and prove that it landed.
	 *
	 * Nothing else on the interface route can create a board file: a hosted app's manifest
	 * arrives listing board ids whose payloads were never downloaded, so this is the single
	 * door. Its post-condition is therefore the strong one — **resolving means the board file
	 * exists locally** — which is what the page lookup and the pre-run gate already assume.
	 *
	 * The gate is `isLocalOnly`, not `isOffline`: an app whose visibility this device has not
	 * learned yet is unknown, not local-only, and must be allowed to ask the server. A refusal
	 * from the hub is the positive evidence that it is local-only.
	 *
	 * A pinned version is deliberately not persisted — no native command writes the
	 * `versions/` layout — so that path returns the remote board and says so.
	 */
	private async materializeBoardFromRemote(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IBoard> {
		const localOnly = await this.backend.isLocalOnly(appId);
		if (localOnly || !this.backend.profile || !this.backend.auth) {
			throw new BoardMaterializationError(boardId, "gated", {
				cause: new Error(
					localOnly
						? `App ${appId} is local-only, so the board cannot be fetched`
						: "No profile or authentication for a remote board fetch",
				),
			});
		}

		let remoteData: IBoard;
		try {
			remoteData = await this.fetchRemoteBoard(appId, boardId, version);
		} catch (error) {
			throw new BoardMaterializationError(boardId, "fetch", { cause: error });
		}

		if (typeof version !== "undefined") {
			return remoteData;
		}

		try {
			await invoke("upsert_board", {
				appId: appId,
				boardId: boardId,
				name: remoteData.name,
				description: remoteData.description,
				logLevel: remoteData.log_level,
				stage: remoteData.stage,
				executionMode: remoteData.execution_mode,
				boardData: remoteData,
			});
		} catch (error) {
			throw new BoardMaterializationError(boardId, "persist", { cause: error });
		}

		// Read it back rather than trusting the write. This is what separates "downloaded" from
		// "on disk", and it warms the local sync client's base for the next read, so the extra
		// round trip is not wasted.
		let materialized: IBoard;
		try {
			materialized = await this.fetchLocalBoard(appId, boardId, version);
		} catch (error) {
			throw new BoardMaterializationError(boardId, "verify", { cause: error });
		}

		await this.recordAppliedRemoteLineage(appId, boardId, remoteData);
		dispatchRemoteBoardApplied(appId, boardId);

		return materialized;
	}

	async getBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		forceFresh?: boolean,
	): Promise<IBoard> {
		let board: IBoard;
		try {
			board = await this.fetchLocalBoard(appId, boardId, version);
		} catch {
			return this.materializeBoardFromRemote(appId, boardId, version);
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
				// Deliver whatever the last interactive edit left in the outbox first, so the remote
				// snapshot fetched below is not older than the board that was just committed here.
				await this.settleOutbox(appId, boardId);
				const pendingSync = await this.backend.getOfflineSyncCommands(
					appId,
					boardId,
				);
				const pendingMutations = pendingSync.filter(
					commandSyncHasPendingMutation,
				);
				if (pendingMutations.length > 0) {
					// Local edits are still queued for the server; the remote snapshot
					// predates them, so the local board is the fresher one.
					console.warn(
						"[BoardState] forceFresh: local board has pending offline sync commands, skipping remote overwrite:",
						{ boardId, pendingBatches: pendingMutations.length },
					);
					return board;
				}

				const remoteData = await this.fetchRemoteBoard(appId, boardId);

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
				const { failed: drainFailed, failure: drainFailure } =
					await this.drainOfflineSyncQueue(appId, boardId);

				if (drainFailed) {
					// Local edits are not on the server yet. Applying the remote snapshot
					// now would clobber them with a board that predates the queued batch.
					this.notifyEditsQueued(appId, boardId, drainFailure);
					return board;
				}

				const remoteData = await this.fetchRemoteBoard(appId, boardId);

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
				// The route reads version_type from the query string only, so a
				// body here silently degrades every Major and Minor to a Patch.
				const remoteData = await fetcher<[number, number, number]>(
					this.backend.profile!,
					`apps/${appId}/board/${boardId}?version_type=${encodeURIComponent(
						versionType,
					)}`,
					{
						method: "PATCH",
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
		let board: IBoard;
		try {
			board = await this.getBoard(
				appId,
				boardId,
				normalizeBoardVersion(payload.version),
				true,
			);
		} catch (error) {
			// Everything below reads the flow to prepare a local run. A user who
			// may run a board but not read it — the normal shape of a published
			// app — gets nothing back from any of it, and the server can run it
			// instead: it holds the board, and resolves permissions, secrets and
			// OAuth on its own.
			if (!(await this.canReachServer(appId))) throw error;
			console.warn(
				"[BoardState] Board unavailable for local execution, running on the server:",
				error,
			);
			return this.executeBoardRemote(
				appId,
				boardId,
				payload,
				streamState,
				eventId,
				cb,
			);
		}

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
		return await this.sequenceBoardMutation(appId, boardId, async () => {
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
			// A background delivery of the last interactive edit must land before the remote undo/redo
			// executes, or the server would undo a different history than the one on screen.
			await this.settleOutbox(appId, boardId);
			await this.assertNoPendingNativeBoardDelivery(appId, boardId);

			// Undo must ship as a single request — a chunked/partial undo would
			// diverge the board — so fail fast instead of hitting a raw HTTP 413.
			const body = JSON.stringify({ commands: commands });
			try {
				chunkCommandsForSync(commands);
			} catch (error) {
				if (!(error instanceof CommandSyncPayloadTooLargeError)) throw error;
				toast.error("Undo batch too large to sync. Undo in smaller steps.");
				throw new Error(
					`Undo batch of ${commands.length} commands exceeds the safe sync limit: ${error.message}`,
					{ cause: error },
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
		});
	}
	async redoBoard(appId: string, boardId: string, commands: IGenericCommand[]) {
		return await this.sequenceBoardMutation(appId, boardId, async () => {
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
			// A background delivery of the last interactive edit must land before the remote undo/redo
			// executes, or the server would undo a different history than the one on screen.
			await this.settleOutbox(appId, boardId);
			await this.assertNoPendingNativeBoardDelivery(appId, boardId);

			// Redo must ship as a single request — a chunked/partial redo would
			// diverge the board — so fail fast instead of hitting a raw HTTP 413.
			const body = JSON.stringify({ commands: commands });
			try {
				chunkCommandsForSync(commands);
			} catch (error) {
				if (!(error instanceof CommandSyncPayloadTooLargeError)) throw error;
				toast.error("Redo batch too large to sync. Redo in smaller steps.");
				throw new Error(
					`Redo batch of ${commands.length} commands exceeds the safe sync limit: ${error.message}`,
					{ cause: error },
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
		});
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
			board?: IBoard | null;
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
			// Template instantiation remaps node and pin IDs and applies catalog schema migrations on
			// the server. Instantiating the template a second time in the native cache would generate a
			// different graph; the next command would then address a local node ID that the API cannot
			// find. New APIs return the exact instantiated board; retain a checked GET for older servers.
			let authoritativeBoard =
				boardUpdate.board?.id === boardId ? boardUpdate.board : undefined;
			if (template && !authoritativeBoard) {
				try {
					const fetchedBoard = await fetcher<IBoard>(
						this.backend.profile,
						`apps/${appId}/board/${boardId}`,
						{ method: "GET" },
						this.backend.auth,
					);
					if (fetchedBoard?.id === boardId) {
						authoritativeBoard = fetchedBoard;
					} else {
						console.warn(
							"Authoritative board read returned no matching board; caching an empty ID-safe placeholder",
							{ boardId },
						);
					}
				} catch (error) {
					console.warn(
						"Failed to read the authoritative instantiated board; caching an empty ID-safe placeholder",
						error,
					);
				}
			}
			// Older deployed APIs return only `{ id }`. Use an intentionally old revision for that
			// compatibility path so the transient readiness cache can never outrank the authoritative
			// remote board during the next sync.
			const authoritativeUpdatedAt = authoritativeBoard
				? (authoritativeBoard.updated_at ?? boardUpdate.updated_at)
				: {
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
				boardData: authoritativeBoard,
				// Never instantiate an online template locally: even the fallback cache must not invent
				// node IDs that are absent from the authoritative server graph.
				template: undefined,
				authoritativeUpdatedAt,
			});
			// Only advance the lineage once the local cache holds this revision;
			// otherwise a refused remote echo could block cache recovery.
			const appliedUpdatedAt = authoritativeBoard
				? (authoritativeBoard.updated_at ?? boardUpdate.updated_at)
				: undefined;
			if (authoritativeBoard && appliedUpdatedAt) {
				await this.recordAppliedRemoteLineage(appId, boardId, {
					updated_at: appliedUpdatedAt,
				});
			}
		} catch (error) {
			// The remote write already succeeded. Do not turn a cache failure into a failed
			// create_app response (and a duplicate retry); the readiness path can still fetch it.
			console.warn("Failed to cache the remote board locally:", error);
		}
	}

	async closeBoard(boardId: string) {
		this.remoteBoardSync.forget(undefined, boardId);
		this.localBoardSync.forget(undefined, boardId);
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

	private notifyEditsQueued(
		appId: string,
		boardId: string,
		failure?: OfflineSyncFailure,
	): void {
		const now = Date.now();
		if (now - lastQueuedEditsToastAt < QUEUED_EDITS_TOAST_DEBOUNCE_MS) return;
		lastQueuedEditsToastAt = now;

		toast.warning("Server sync is incomplete — queued edits were kept.", {
			description: describeOfflineSyncFailure(failure),
			// Two actions plus a transport error do not fit the default toast width.
			style: {
				"--width": "min(28rem, calc(100vw - 2rem))",
			} as CSSProperties,
			action: {
				label: "Retry",
				onClick: () => {
					void this.retryOfflineSync(appId, boardId);
				},
			},
			// A queue the server permanently refuses cannot be retried out of. This opens the
			// recovery dialog rather than resetting directly — discarding an edit needs consent.
			cancel: {
				label: "Fetch from server",
				onClick: () => dispatchBoardSyncRecoveryRequest(appId, boardId),
			},
		});
		dispatchBoardSyncChanged(appId, boardId);
	}

	/**
	 * Restate `on_update`-derived node state that a queued batch never captured.
	 *
	 * Batches authored before the apply planner shipped that state reference dynamic pin ids that
	 * exist on no other machine, so the Hub rejects them permanently. The local board still holds
	 * those ids, so the batch can be made replayable without losing or reordering a mutation.
	 * Returns undefined when nothing needs repair or the local board cannot supply every pin.
	 */
	private async repairUnreplayableChunks(
		appId: string,
		boardId: string,
		chunks: IGenericCommand[][],
	): Promise<IGenericCommand[][] | undefined> {
		const needsRepair = chunks.some(
			(chunk) => findUnresolvedPinReferences(chunk).missing.size > 0,
		);
		if (!needsRepair) return undefined;

		let localNodes: Record<string, INode>;
		try {
			const localBoard = await invoke<IBoard>("get_board", { appId, boardId });
			localNodes = localBoard.nodes ?? {};
		} catch (error) {
			console.warn(
				"Cannot repair a queued board batch without the local board:",
				error,
			);
			return undefined;
		}

		const repaired = chunks.map(
			(chunk) => repairUnreplayableCommandBatch(chunk, localNodes) ?? chunk,
		);
		if (repaired.every((chunk, index) => chunk === chunks[index])) {
			return undefined;
		}
		console.warn(
			"Restated derived node state so a queued board batch can be replayed remotely:",
			{ appId, boardId },
		);
		return repaired;
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
	): Promise<OfflineSyncDrainResult> {
		const key = `${appId}\u001f${boardId}`;
		const active = this.offlineSyncDrains.get(key);
		if (active) return await active;
		const drain = this.drainOfflineSyncQueueExclusive(appId, boardId).finally(
			() => {
				if (this.offlineSyncDrains.get(key) === drain) {
					this.offlineSyncDrains.delete(key);
				}
			},
		);
		this.offlineSyncDrains.set(key, drain);
		return await drain;
	}

	private async drainOfflineSyncQueueExclusive(
		appId: string,
		boardId: string,
	): Promise<OfflineSyncDrainResult> {
		let failure: OfflineSyncFailure | undefined;
		const unsyncedCommands = await this.backend.getOfflineSyncCommands(
			appId,
			boardId,
		);
		const transportProfile = this.backend.profile;
		const transportAuth = this.backend.auth;
		if (!transportProfile || !transportAuth) {
			return {
				failed: unsyncedCommands.some(commandSyncHasPendingMutation),
				pushedBatches: 0,
			};
		}
		// Freeze one authenticated destination for the complete ordered drain. A profile
		// switch between legacy recovery chunks must never route the tail to another Hub.
		const transportIdentity: CommandSyncRemoteIdentity = {
			remoteIdentityVersion: 1,
			remoteProfileId: transportProfile.id,
			remotePrincipalId: transportAuth.user?.profile.sub,
			remoteHub: transportProfile.hub,
		};
		let failed = false;
		let pushedBatches = 0;

		for (const commandSync of unsyncedCommands) {
			const hasPendingMutation = commandSyncHasPendingMutation(commandSync);
			if (hasPendingMutation && commandSync.remoteIdentityVersion !== 1) {
				const message =
					"A queued edit from an older desktop version has no recorded account or Hub. Use Retry queued edits to confirm its owner.";
				console.error(message);
				failure = { message };
				failed = true;
				break;
			}
			const remoteIdentity = evaluateCommandSyncRemoteIdentity(
				commandSync,
				transportIdentity,
			);
			if (!remoteIdentity.apply) {
				if (!hasPendingMutation) {
					console.warn(
						"Leaving a completed board-delivery tombstone with its original remote identity:",
						remoteIdentity.refusalReason,
					);
					continue;
				}
				const message = `Queued edits belong to a different remote identity (${remoteIdentity.refusalReason}). Sign in to the original account and Hub.`;
				console.error(
					"Refusing to drain a board mutation through a different remote identity:",
					remoteIdentity.refusalReason,
				);
				failure = { message };
				failed = true;
				break;
			}
			// A later row was produced from local state that already includes this one.
			// Silently deleting an expired prefix would let dependent edits overtake it.
			if (
				hasPendingMutation &&
				commandSync.createdAt.getTime() <
					Date.now() - OFFLINE_SYNC_COMMAND_MAX_AGE_MS &&
				!commandSync.blockedReason &&
				!commandSync.idempotencyKey?.startsWith("flowpilot-board-edit:")
			) {
				const blockedReason =
					"This queued board edit is older than the automatic replay window. It was retained because later edits may depend on it; restore or rebase the board before syncing again.";
				console.error(blockedReason, commandSync.commandId);
				await this.backend.blockOfflineSyncCommand(
					commandSync.commandId,
					blockedReason,
				);
				failure = { message: blockedReason };
				failed = true;
				break;
			}
			if (commandSync.blockedReason) {
				console.error(
					"Board sync is blocked behind a payload that exceeds the command transport:",
					commandSync.blockedReason,
				);
				failure = { message: commandSync.blockedReason };
				failed = true;
				break;
			}

			try {
				const acknowledgeRemoteReceipt = async (receiptKey: string) => {
					await fetcher(
						transportProfile,
						`apps/${appId}/board/${boardId}`,
						{
							method: "POST",
							headers: {
								"FlowLike-Idempotency-Ack": receiptKey,
							},
							body: JSON.stringify({ commands: [] }),
						},
						transportAuth,
					);
				};
				const baseKey =
					commandSync.idempotencyKey ?? `offline-sync:${commandSync.commandId}`;
				const isFlowPilotDelivery =
					commandSync.deferReceiptAckUntilNativeTerminal === true ||
					baseKey.startsWith("flowpilot-board-edit:");

				// The server mutation was accepted and IndexedDB advanced before a prior
				// crash. Old FlowPilot rows are migrated to retained evidence because ACKing
				// before every native replay authority is gone can reopen duplicate delivery.
				if (commandSync.pendingReceiptAck) {
					if (isFlowPilotDelivery) {
						await this.backend.deferOfflineSyncReceiptAck(
							commandSync.commandId,
							commandSync.pendingReceiptAck,
						);
					} else {
						await acknowledgeRemoteReceipt(commandSync.pendingReceiptAck);
						await this.backend.completeOfflineSyncReceiptAck(
							commandSync.commandId,
							commandSync.pendingReceiptAck,
						);
					}
				}
				let chunks = commandSync.chunks;
				if (!chunks) {
					try {
						chunks = chunkLegacyCommandsForRecovery(commandSync.commands ?? []);
					} catch (error) {
						if (!(error instanceof CommandSyncPayloadTooLargeError))
							throw error;
						const blockedReason = `${error.message} This legacy recovery tail cannot use the atomic command endpoint; restore the Hub board from an authoritative snapshot.`;
						await this.backend.blockOfflineSyncCommand(
							commandSync.commandId,
							blockedReason,
						);
						throw new Error(blockedReason, { cause: error });
					}
					// Persist the exact partition and stable key before the first request. After
					// this transaction, every crash/retry reuses the same key-to-digest mapping.
					await this.backend.migrateLegacyOfflineSyncCommand(
						commandSync.commandId,
						chunks,
						baseKey,
					);
				}
				// A completed FlowPilot tombstone proves the remote/outbox handoff without
				// deleting the server replay evidence or blocking later semantic mutations.
				if (chunks.length === 0) {
					if (isFlowPilotDelivery) {
						pushedBatches += 1;
						continue;
					}
					await this.backend.clearOfflineSyncCommands(
						commandSync.commandId,
						appId,
						boardId,
					);
					continue;
				}
				// Only a batch the server has provably never seen may be rewritten: the durable
				// receipt is keyed on the payload digest, so repairing a partially delivered batch
				// would turn every later retry into an idempotency conflict.
				if (
					(commandSync.chunkOffset ?? 0) === 0 &&
					!commandSync.pendingReceiptAck
				) {
					const repaired = await this.repairUnreplayableChunks(
						appId,
						boardId,
						chunks,
					);
					if (repaired) {
						await this.backend.repairOfflineSyncCommand(
							commandSync.commandId,
							repaired,
						);
						chunks = repaired;
					}
				}
				const initialOffset = commandSync.chunkOffset ?? 0;
				for (const [chunkIndex, chunk] of chunks.entries()) {
					try {
						chunkCommandsForSync(chunk);
					} catch (error) {
						if (!(error instanceof CommandSyncPayloadTooLargeError))
							throw error;
						const blockedReason = `${error.message} The persisted outbox payload requires snapshot recovery.`;
						await this.backend.blockOfflineSyncCommand(
							commandSync.commandId,
							blockedReason,
						);
						throw new Error(blockedReason, { cause: error });
					}
					const receiptKey = `${baseKey}:${initialOffset + chunkIndex}`;
					await fetcher(
						transportProfile,
						`apps/${appId}/board/${boardId}`,
						{
							method: "POST",
							headers: {
								"Idempotency-Key": receiptKey,
							},
							body: JSON.stringify({
								commands: chunk,
							}),
						},
						transportAuth,
					);
					// Persist exact progress after every accepted chunk. A lost checkpoint can
					// only replay the same digest/key, which the board-persisted server marker
					// handles idempotently.
					await this.backend.checkpointOfflineSyncCommand(
						commandSync.commandId,
						chunks.slice(chunkIndex + 1),
						initialOffset + chunkIndex + 1,
						baseKey,
						receiptKey,
						isFlowPilotDelivery,
					);
					if (!isFlowPilotDelivery) {
						// Only the durable checkpoint authorizes ordinary receipt deletion.
						// FlowPilot retains both server evidence and a nonblocking client tombstone.
						await acknowledgeRemoteReceipt(receiptKey);
						await this.backend.completeOfflineSyncReceiptAck(
							commandSync.commandId,
							receiptKey,
						);
					}
				}
				pushedBatches += 1;
				console.log("Executed offline sync command:", commandSync.commandId);
			} catch (e) {
				// Keep the batch queued and stop: later batches must not overtake it. The reason is
				// persisted first — a rejected payload and a dropped connection are indistinguishable
				// from the queue alone, and only the former needs the user to act.
				const status = e instanceof ApiResponseError ? e.status : undefined;
				const message = getErrorMessage(e, "Unknown sync transport error");
				console.warn(
					"Failed to push offline sync command; keeping it queued:",
					{ commandId: commandSync.commandId, status, message },
					e,
				);
				await this.backend.recordOfflineSyncFailure(commandSync.commandId, {
					status,
					message,
				});
				failure = { status, message };
				failed = true;
				break;
			}
		}

		if (!failed && pushedBatches > 0) {
			await this.recordLineageAfterPush(appId, boardId);
		}

		if (pushedBatches > 0 || failed) dispatchBoardSyncChanged(appId, boardId);

		return { failed, pushedBatches, failure };
	}

	/** The account/Hub a drain would use right now — undefined while signed out. */
	private currentTransportIdentity(): CommandSyncRemoteIdentity | undefined {
		if (!this.backend.profile) return undefined;
		return {
			remoteIdentityVersion: 1,
			remoteProfileId: this.backend.profile.id,
			remotePrincipalId: this.backend.auth?.user?.profile.sub,
			remoteHub: this.backend.profile.hub,
		};
	}

	async getBoardSyncStatus(
		appId: string,
		boardId: string,
	): Promise<IBoardSyncStatus> {
		try {
			const rows = await this.backend.getOfflineSyncCommands(appId, boardId);
			const summary = summarizeBoardSyncQueue(
				rows,
				this.currentTransportIdentity() ?? {},
			);
			return { supported: true, ...summary };
		} catch (error) {
			// The status surface must never be the reason a board fails to render.
			console.warn("Failed to read the board sync queue:", error);
			return {
				supported: true,
				pendingBatches: 0,
				blockedBatches: 0,
				partiallyDeliveredBatches: 0,
				entries: [],
			};
		}
	}

	async exportBoardSyncArchive(
		appId: string,
		boardId: string,
	): Promise<unknown[]> {
		return await this.backend.listOfflineSyncArchive(appId, boardId);
	}

	/**
	 * Make the server's board authoritative again for a client whose outbox cannot drain.
	 *
	 * A queued batch bound to another account/Hub, or one the server permanently rejects, blocks
	 * every remote→local path for that board and every further edit. Delivery is attempted first,
	 * the snapshot is fetched before anything is destroyed, and only then — with explicit consent —
	 * are the undelivered batches archived out of the queue.
	 */
	async resetBoardFromServer(
		appId: string,
		boardId: string,
		options: { discardQueuedEdits: boolean },
	): Promise<IBoardServerResetResult> {
		const app = await invoke<{ visibility?: IAppVisibility }>("get_app", {
			appId,
		});
		if ((app.visibility ?? IAppVisibility.Offline) === IAppVisibility.Offline) {
			throw new Error(
				"This app is local-only, so there is no server board to fetch. Queued edits are the only copy of those changes.",
			);
		}
		// Freeze one authenticated destination for the whole reset: a profile switch midway must
		// not read the board from a different Hub than the one just validated.
		const transportProfile = this.backend.profile;
		const transportAuth = this.backend.auth;
		if (!transportProfile || !transportAuth?.user?.profile.sub) {
			throw new Error(
				"Sign in to fetch the server board — the account and Hub to read it from are unavailable.",
			);
		}
		await this.assertNoPendingNativeBoardDelivery(appId, boardId);

		return await this.sequenceBoardMutation(appId, boardId, async () => {
			// Anything the server will still accept must be delivered before it can be discarded.
			const { pushedBatches } = await this.drainOfflineSyncQueue(
				appId,
				boardId,
			);

			const queued = await this.backend.getOfflineSyncCommands(appId, boardId);
			const discardable = selectDiscardableSyncRows(queued);
			if (discardable.length > 0 && !options.discardQueuedEdits) {
				throw new BoardSyncDiscardRequiredError(
					await this.getBoardSyncStatus(appId, boardId),
				);
			}

			// Fetch before discarding: a transport failure then leaves the queue exactly as it was.
			const remoteData = await this.fetchRemoteBoard(
				appId,
				boardId,
				undefined,
				transportProfile,
				transportAuth,
			);
			if (!remoteData) {
				throw new Error(
					`The server returned no board for ${boardId}; nothing was discarded.`,
				);
			}

			const discardedBatches = await this.backend.archiveOfflineSyncCommands(
				appId,
				boardId,
				discardable.map((entry) => entry.commandId),
				"Discarded by an explicit fetch-from-server board reset.",
			);
			// The recorded lineage can be newer than every server revision (local clock skew), which
			// would make the snapshot fetched above be refused by the next background sync.
			await this.backend.clearBoardLineage(appId, boardId);

			const localBoard = await invoke<IBoard>("get_board", {
				appId,
				boardId,
			}).catch(() => undefined);
			const { merged } = await mergeBoardOffThread(remoteData, localBoard);

			await invoke("upsert_board", {
				appId,
				boardId,
				name: merged.name,
				description: merged.description,
				logLevel: merged.log_level,
				stage: merged.stage,
				executionMode: merged.execution_mode,
				boardData: merged,
				authoritativeUpdatedAt: remoteData.updated_at,
			});
			await this.recordAppliedRemoteLineage(appId, boardId, remoteData);

			this.backend.queryClient?.setQueryData(
				[this.getBoard.name || "backendFn", appId, boardId],
				merged,
			);
			void this.backend.queryClient?.invalidateQueries({
				queryKey: [this.getBoards.name || "backendFn", appId],
			});
			dispatchRemoteBoardApplied(appId, boardId);
			dispatchBoardSyncChanged(appId, boardId);

			console.warn("Reset board from the server copy:", {
				appId,
				boardId,
				pushedBatches,
				discardedBatches,
			});

			return { board: merged, discardedBatches, pushedBatches };
		});
	}

	async retryOfflineSync(
		appId: string,
		boardId: string,
	): Promise<{ pushedBatches: number; remainingBatches: number }> {
		const countRemaining = async () =>
			(await this.backend.getOfflineSyncCommands(appId, boardId)).filter(
				commandSyncHasPendingMutation,
			).length;

		const app = await invoke<{ visibility?: IAppVisibility }>("get_app", {
			appId,
		});
		const isLocalOnly =
			(app.visibility ?? IAppVisibility.Offline) === IAppVisibility.Offline;
		if (isLocalOnly) {
			const queued = await this.backend.getOfflineSyncCommands(appId, boardId);
			const pending = queued.filter(commandSyncHasPendingMutation);
			for (const entry of queued.filter(
				(entry) => !commandSyncHasPendingMutation(entry),
			)) {
				await this.backend.clearOfflineSyncCommands(
					entry.commandId,
					appId,
					boardId,
				);
			}
			if (pending.length > 0) {
				toast.error(
					"Queued remote edits were retained because changing app visibility cannot safely discard an ordered mutation.",
				);
			}
			return { pushedBatches: 0, remainingBatches: pending.length };
		}
		const principal = this.backend.auth?.user?.profile.sub;
		if (!this.backend.profile || !this.backend.auth || !principal) {
			toast.error("Cannot sync queued edits while offline or signed out.");
			return { pushedBatches: 0, remainingBatches: await countRemaining() };
		}

		const queued = await this.backend.getOfflineSyncCommands(appId, boardId);
		const ownerless = queued.filter(
			(entry) =>
				commandSyncHasPendingMutation(entry) &&
				entry.remoteIdentityVersion !== 1,
		);
		if (ownerless.length > 0) {
			const approved = await confirm(
				`This board has ${ownerless.length} queued edit ${ownerless.length === 1 ? "batch" : "batches"} from an older desktop version that did not record its account or Hub. Retry only if this is the same account and Hub where the edit was originally made. Continue?`,
			);
			if (!approved) {
				return {
					pushedBatches: 0,
					remainingBatches: await countRemaining(),
				};
			}
			const identity: CommandSyncRemoteIdentity = {
				remoteIdentityVersion: 1,
				remoteProfileId: this.backend.profile.id,
				remotePrincipalId: principal,
				remoteHub: this.backend.profile.hub,
			};
			for (const entry of ownerless) {
				await this.backend.bindLegacyOfflineSyncCommand(
					entry.commandId,
					identity,
				);
			}
		}

		const { failed, pushedBatches, failure } = await this.drainOfflineSyncQueue(
			appId,
			boardId,
		);
		const remainingBatches = await countRemaining();

		if (failed) {
			toast.error(
				`Sync retry failed — ${remainingBatches} edit ${remainingBatches === 1 ? "batch is" : "batches are"} still queued.`,
				{ description: describeOfflineSyncFailure(failure) },
			);
		} else if (pushedBatches > 0) {
			toast.success("Queued edits synced to the server.");
		} else {
			toast.info("No queued edits to sync.");
		}

		dispatchBoardSyncChanged(appId, boardId);
		return { pushedBatches, remainingBatches };
	}

	/**
	 * Journal one logical mutation, then push it to the server as one atomic request.
	 *
	 * FlowScript setup, dynamic-pin updates, function targets, and connections are one
	 * transaction. The API persists every request independently, so size-based splitting
	 * would make a failed tail observable as a nodes-only board. Older multi-chunk outbox
	 * rows remain readable solely so an already-partial delivery can finish recovery.
	 */
	private async syncExecutedCommandsToServer(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
		idempotencyKey?: string,
		remoteIdentity?: CommandSyncRemoteIdentity,
		{ awaitDelivery = true }: { awaitDelivery?: boolean } = {},
	): Promise<{ deliveryComplete: boolean; blockedReason?: string }> {
		if (commands.length === 0) return { deliveryComplete: true };
		const durableIdempotencyKey = idempotencyKey ?? `board-sync:${createId()}`;

		// Every caller captures this target before its native mutation. Do not re-read app
		// visibility afterwards: a concurrent visibility change must not discard an edit that
		// was committed for a remote destination.
		const isLocalOnly = !remoteIdentity;
		if (isLocalOnly) {
			const queued = await this.backend.getOfflineSyncCommands(appId, boardId);
			for (const entry of queued.filter(
				(entry) => !commandSyncHasPendingMutation(entry),
			)) {
				await this.backend.clearOfflineSyncCommands(
					entry.commandId,
					appId,
					boardId,
				);
			}
			return { deliveryComplete: true };
		}
		if (!remoteIdentity?.remotePrincipalId) {
			throw new Error(
				"Remote board delivery requires the account and Hub identity captured before the native mutation.",
			);
		}

		let chunks: IGenericCommand[][];
		try {
			chunks = chunkCommandsForSync(commands);
		} catch (error) {
			if (!(error instanceof CommandSyncPayloadTooLargeError)) throw error;
			const blockedReason = `${error.message} The exact locally committed payload is retained in the durable sync outbox, but remote delivery requires a larger/snapshot transport.`;
			await this.backend.pushOfflineSyncCommand(
				appId,
				boardId,
				[],
				durableIdempotencyKey,
				0,
				commands,
				blockedReason,
				remoteIdentity,
			);
			toast.error(blockedReason);
			return { deliveryComplete: false, blockedReason };
		}

		// Every online mutation enters IndexedDB before the first network attempt. This makes the
		// outbox the single ordering source for direct sends, retries, crashes, and concurrent edits.
		await this.backend.pushOfflineSyncCommand(
			appId,
			boardId,
			chunks,
			durableIdempotencyKey,
			0,
			undefined,
			undefined,
			remoteIdentity,
		);

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			this.notifyEditsQueued(appId, boardId, {
				message:
					"The edit is journaled locally but no signed-in Hub destination is available yet.",
			});
			return { deliveryComplete: true };
		}

		if (!awaitDelivery) {
			void this.deliverOutbox(appId, boardId).catch((error: unknown) => {
				console.error("[BoardState] background outbox delivery failed:", error);
			});
			return { deliveryComplete: true };
		}
		await this.deliverOutbox(appId, boardId);
		return { deliveryComplete: true };
	}

	/**
	 * Drain the outbox to the hub, then once more if a drain that was already running snapshotted
	 * the queue before the latest journal write. Announces delivery to the window so the canvas
	 * can tell peers to refetch, and surfaces a failure as the queued-edits toast.
	 */
	private async deliverOutbox(appId: string, boardId: string): Promise<void> {
		let drain = await this.drainOfflineSyncQueue(appId, boardId);
		if (!drain.failed) {
			const remaining = await this.backend.getOfflineSyncCommands(
				appId,
				boardId,
			);
			if (remaining.some(commandSyncHasPendingMutation)) {
				drain = await this.drainOfflineSyncQueue(appId, boardId);
			}
		}
		if (drain.failed) {
			this.notifyEditsQueued(appId, boardId, drain.failure);
			return;
		}
		dispatchBoardDelivered(appId, boardId);
	}

	/**
	 * Wait until nothing is left in the board's outbox that a remote-mutating action could
	 * overtake. No-op when the outbox is empty; a failed drain leaves the entries queued and the
	 * caller proceeds against a hub that is behind — the same situation an offline period leaves.
	 */
	private async settleOutbox(appId: string, boardId: string): Promise<void> {
		const pending = await this.backend.getOfflineSyncCommands(appId, boardId);
		if (!pending.some(commandSyncHasPendingMutation)) return;
		await this.deliverOutbox(appId, boardId);
	}

	async executeCommand(
		appId: string,
		boardId: string,
		command: IGenericCommand,
		options?: IBoardMutationOptions,
	): Promise<IGenericCommand> {
		const executed = await this.executeCommands(
			appId,
			boardId,
			[command],
			options,
		);
		return executed[0] ?? command;
	}

	/**
	 * Local commit first, in one IPC round trip that also returns the board diff against what the
	 * webview holds (`sync`), so the caller can skip its refetch. Hub delivery is journaled to the
	 * outbox before returning — that is the durability guarantee — and then drained in the
	 * background: the outbox is the single ordering source and drains are single-flight per board,
	 * so a later edit can never overtake this one, and every remote-mutating path settles the
	 * outbox before it acts (`settleOutbox`).
	 */
	async executeCommands(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
		options?: IBoardMutationOptions,
	): Promise<IGenericCommand[]> {
		return await this.sequenceBoardMutation(appId, boardId, async () => {
			const remoteIdentity = await this.remoteBoardDeliveryIdentity(
				appId,
				boardId,
			);
			if (remoteIdentity) {
				// Reject an obviously unsyncable caller payload before committing it locally. Some
				// commands acquire undo metadata during execution, so the returned commands are checked
				// and durably journaled again below.
				chunkCommandsForSync(commands);
			}
			const sync = this.localBoardSync.syncRequest(appId, boardId, undefined);
			// The native side returns the executed commands plus any node state `on_update` derived
			// from them. All of it must reach the server as one batch, or a later ConnectPin will
			// reference a dynamic pin id the Hub never minted.
			const result = await invoke<{
				commands: IGenericCommand[];
				sync?: IBoardSyncResponse | null;
			}>("execute_commands", {
				appId,
				boardId,
				commands,
				sync,
			});
			const executedCommands = result.commands;

			if (sync && result.sync && options?.onBoard) {
				const board = this.localBoardSync.ingest(
					appId,
					boardId,
					undefined,
					sync,
					result.sync,
				);
				if (board) {
					await this.presignMediaComments(
						appId,
						boardId,
						board,
						!remoteIdentity,
					);
					options.onBoard(board);
				}
			}

			await this.syncExecutedCommandsToServer(
				appId,
				boardId,
				executedCommands,
				undefined,
				remoteIdentity,
				{ awaitDelivery: false },
			);
			return executedCommands;
		});
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
		return await this.sequenceBoardMutation(appId, boardId, async () => {
			const remoteIdentity = await this.remoteBoardDeliveryIdentity(
				appId,
				boardId,
			);
			let result: IApplyFlowScriptResponse;
			try {
				result = await invoke<IApplyFlowScriptResponse>("apply_flowscript", {
					appId,
					boardId,
					flowscript,
					currentLayer,
					catalogNodes: getAppPackageCatalogNodes(catalogNodes),
					allowDeletions,
					scopeAnchors,
					module,
				});
			} catch (error) {
				void this.reportFlowScriptApplyFailure({
					appId,
					boardId,
					currentLayer,
					allowDeletions,
					flowscript,
					origin,
					outcome: "error",
					errorMessage: getErrorMessage(
						error,
						"Unknown FlowScript apply error",
					),
					diagnostics: [],
					corrections: [],
					commandCount: 0,
				});
				throw error;
			}

			// Classified on the native result: a blocked remote delivery below appends its own
			// diagnostic, and that is a sync failure, not the user's edit going wrong.
			const outcome = flowScriptApplyOutcome(
				result.commands.length,
				result.diagnostics.length,
			);
			if (outcome) {
				void this.reportFlowScriptApplyFailure({
					appId,
					boardId,
					currentLayer,
					allowDeletions,
					flowscript,
					origin,
					outcome,
					diagnostics: result.diagnostics,
					corrections: result.corrections ?? [],
					commandCount: result.commands.length,
				});
			}

			if (result.commands.length > 0) {
				const sync = await this.syncExecutedCommandsToServer(
					appId,
					boardId,
					result.commands,
					undefined,
					remoteIdentity,
				);
				if (!sync.deliveryComplete) {
					return {
						...result,
						diagnostics: [
							...result.diagnostics,
							sync.blockedReason ??
								"FlowScript applied locally, but its atomic remote delivery is blocked.",
						],
					};
				}
			}
			return result;
		});
	}

	/**
	 * Report an apply that did not do what the user asked, so the source that produced it can be
	 * reviewed. Best-effort in every direction: the source is redacted natively first so nothing
	 * raw leaves the machine, a signed-out user reports nothing, and a failed report is swallowed —
	 * losing a capture must never cost someone their edit. The typed-IR commit path holds only a
	 * compiler token, never the draft source, so `flowscript` may be absent there; a supplied but
	 * empty source still reports nothing.
	 */
	private async reportFlowScriptApplyFailure(failure: {
		appId: string;
		boardId: string;
		currentLayer?: string;
		allowDeletions: boolean;
		flowscript?: string;
		outcome: FlowScriptApplyOutcome;
		origin: FlowScriptApplyOrigin;
		errorMessage?: string;
		diagnostics: string[];
		corrections: string[];
		commandCount: number;
	}): Promise<void> {
		const { profile, auth } = this.backend;
		if (!profile || !auth) return;

		try {
			let flowscript = "";
			if (failure.flowscript !== undefined) {
				if (!failure.flowscript.trim()) return;
				const redacted = await invoke<{ flowscript: string }>(
					"redact_flowscript",
					{ flowscript: failure.flowscript },
				);
				if (!redacted.flowscript.trim()) return;
				flowscript = redacted.flowscript;
			}

			const report: IFlowScriptApplyFailureReport = {
				app_id: failure.appId,
				board_id: failure.boardId,
				layer_id: failure.currentLayer,
				outcome: failure.outcome,
				origin: failure.origin,
				flowscript,
				error_message: failure.errorMessage,
				diagnostics: failure.diagnostics,
				corrections: failure.corrections,
				command_count: failure.commandCount,
				allow_deletions: failure.allowDeletions,
				app_version: await getVersion().catch(() => undefined),
				platform: desktopPlatform(),
			};

			await fetcher(
				profile,
				FLOWSCRIPT_APPLY_FAILURE_PATH,
				{ method: "POST", body: JSON.stringify(report) },
				auth,
			);
		} catch (error) {
			console.debug("[applyFlowScript] failure report skipped", error);
		}
	}

	/**
	 * Capture a terminal typed-IR commit outcome, classified on the native result before remote
	 * delivery so a blocked sync is never miscounted as the agent's edit going wrong. Replays and
	 * already-delivered claims are earlier successes, not failures.
	 */
	private captureFlowIrCommitFailure(
		appId: string,
		token: FlowIrCommitToken,
		result: IApplyFlowIrCommitResponse,
	): void {
		if (result.replayed || result.code === "IR_COMMIT_DELIVERY_FINALIZED")
			return;
		// A user declining the native destructive dialog is a choice, not an agent failure.
		if (result.code === "IR_COMMIT_DESTRUCTIVE_APPROVAL_DENIED") return;
		const outcome =
			result.status === "applied"
				? flowScriptApplyOutcome(
						result.commands.length,
						result.diagnostics.length,
					)
				: result.status === "error"
					? "error"
					: "blocked";
		if (!outcome) return;
		void this.reportFlowScriptApplyFailure({
			appId,
			boardId: token.board_id,
			allowDeletions: token.requires_destructive_approval ?? false,
			origin: "agent",
			outcome,
			errorMessage:
				result.status === "applied"
					? undefined
					: result.code
						? `${result.code}: ${result.message}`
						: result.message,
			diagnostics: result.diagnostics,
			corrections: result.corrections ?? [],
			commandCount: result.commands.length,
		});
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

	async getFlowScriptScoped(
		appId: string,
		boardId: string,
		nodeIds: string[],
		anchors = true,
	): Promise<IScopedFlowScriptResponse> {
		try {
			return await invoke<IScopedFlowScriptResponse>("get_flowscript_scoped", {
				appId,
				boardId,
				nodeIds,
				anchors,
			});
		} catch {
			const isOffline = await this.backend.isOffline(appId);
			if (isOffline || !this.backend.profile || !this.backend.auth) {
				throw new Error(`Board not found: ${boardId}`);
			}
			const params = new URLSearchParams();
			params.set("anchors", String(anchors));
			params.set("node_ids", nodeIds.join(","));
			const response = await fetcher<{
				flowscript: string;
				scope_anchors?: string[];
			}>(
				this.backend.profile,
				`apps/${appId}/board/${boardId}/flowscript?${params}`,
				{ method: "GET" },
				this.backend.auth,
			);
			return {
				flowscript: response.flowscript,
				scope_anchors: response.scope_anchors ?? [],
			};
		}
	}

	async getFlowScriptFile(
		appId: string,
		boardId: string,
		file: string,
		anchors = true,
	): Promise<IScopedFlowScriptResponse> {
		try {
			return await invoke<IScopedFlowScriptResponse>("get_flowscript_file", {
				appId,
				boardId,
				file,
				anchors,
			});
		} catch {
			const isOffline = await this.backend.isOffline(appId);
			if (isOffline || !this.backend.profile || !this.backend.auth) {
				throw new Error(`Board not found: ${boardId}`);
			}
			const params = new URLSearchParams();
			params.set("anchors", String(anchors));
			params.set("file", file);
			const response = await fetcher<{
				flowscript: string;
				scope_anchors?: string[];
			}>(
				this.backend.profile,
				`apps/${appId}/board/${boardId}/flowscript?${params}`,
				{ method: "GET" },
				this.backend.auth,
			);
			return {
				flowscript: response.flowscript,
				scope_anchors: response.scope_anchors ?? [],
			};
		}
	}

	async formatFlowScript(
		_appId: string,
		_boardId: string,
		flowscript: string,
		anchors = true,
	): Promise<string> {
		return await invoke<string>("format_flowscript", {
			flowscript,
			anchors,
		});
	}

	async lintFlowScript(flowscript: string): Promise<IFlowScriptDiagnostic[]> {
		return await invoke<IFlowScriptDiagnostic[]>("lint_flowscript", {
			flowscript,
		});
	}

	async checkFlowScriptReconcile(
		appId: string,
		boardId: string,
		flowscript: string,
		scopeAnchors?: string[],
		module?: string,
	): Promise<ICheckFlowScriptReconcileResponse> {
		return await invoke<ICheckFlowScriptReconcileResponse>(
			"check_flowscript_reconcile",
			{ appId, boardId, flowscript, scopeAnchors, module },
		);
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
		nested?: boolean,
		readOnly?: boolean,
		toolContext?: CopilotToolContext,
		requestId?: string,
		rawUserPrompt?: string,
		appId?: string,
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
			currentCanvasSettings,
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
			appId,
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
		deliveryId?: string,
	): Promise<IApplyFlowIrCommitResponse> {
		return await this.sequenceBoardMutation(appId, token.board_id, async () => {
			const currentIdentity = await this.remoteBoardDeliveryIdentity(
				appId,
				token.board_id,
			);
			const durableDeliveryId = flowIrCommitDeliveryId(token);
			if (deliveryId && deliveryId !== durableDeliveryId) {
				throw new Error(
					"FlowPilot delivery identity must match its immutable compiler claim.",
				);
			}
			let deliveryIdentity = currentIdentity;
			const owningJob = (
				await this.listBoardEditJobs(appId, token.board_id, true)
			).find(
				(job) =>
					job.token.draft_id === token.draft_id &&
					job.token.revision === token.revision &&
					job.token.base_fingerprint === token.base_fingerprint &&
					job.token.claim_id === token.claim_id,
			);
			if (currentIdentity) {
				// A direct compatibility call may replay a receipt that was already committed by
				// the durable job, but it must never be the authority that mutates a remote board.
				// Otherwise a renderer crash between native commit and outbox journaling could send
				// the edit later through a different account or Hub.
				if (!owningJob) {
					throw new Error(
						"Remote FlowPilot edits require a durable board-edit review before native Apply. Regenerate the review and retry it from Pending edits.",
					);
				}
				if (owningJob.phase !== "applied_pending_delivery") {
					throw new Error(
						`Remote FlowPilot Apply must resolve its durable review before receipt delivery (current phase: ${owningJob.phase}).`,
					);
				}
				if (
					!owningJob.remoteProfileId ||
					!owningJob.remotePrincipalId ||
					!owningJob.remoteHub
				) {
					throw new Error(
						"The applied FlowPilot review has no complete durable remote owner. It was not delivered; dismiss and regenerate it while signed in.",
					);
				}
				const boundIdentity: CommandSyncRemoteIdentity = {
					remoteIdentityVersion: 1,
					remoteProfileId: owningJob.remoteProfileId,
					remotePrincipalId: owningJob.remotePrincipalId,
					remoteHub: owningJob.remoteHub,
				};
				const ownerMatch = evaluateCommandSyncRemoteIdentity(
					boundIdentity,
					currentIdentity,
				);
				if (!ownerMatch.apply) {
					throw new Error(
						`This FlowPilot review belongs to another remote identity: ${ownerMatch.refusalReason}.`,
					);
				}
				deliveryIdentity = boundIdentity;
			}
			let result: IApplyFlowIrCommitResponse;
			try {
				result = await invoke<IApplyFlowIrCommitResponse>(
					"flowpilot_apply_flow_ir_commit",
					{
						appId,
						token,
					},
				);
			} catch (error) {
				void this.reportFlowScriptApplyFailure({
					appId,
					boardId: token.board_id,
					allowDeletions: token.requires_destructive_approval ?? false,
					origin: "agent",
					outcome: "error",
					errorMessage: getErrorMessage(
						error,
						"Unknown typed workflow apply error",
					),
					diagnostics: [],
					corrections: [],
					commandCount: 0,
				});
				throw error;
			}
			this.captureFlowIrCommitFailure(appId, token, result);
			if (result.status !== "applied" || result.commands.length === 0) {
				return result;
			}

			try {
				const sync = await this.syncExecutedCommandsToServer(
					appId,
					token.board_id,
					result.commands,
					durableDeliveryId,
					deliveryIdentity,
				);
				if (!sync.deliveryComplete) {
					const warning =
						sync.blockedReason ??
						"Typed workflow applied locally, but its remote delivery remains blocked.";
					return {
						...result,
						delivery_complete: false,
						diagnostics: [...result.diagnostics, warning],
					};
				}
			} catch (error) {
				// Native apply has already committed the exact batch, but renderer delivery is
				// deliberately incomplete until either the server or durable outbox accepts it.
				const warning = `Typed workflow applied locally; remote synchronization must retry: ${getErrorMessage(error, "Unknown sync error")}`;
				console.error(warning, error);
				return {
					...result,
					delivery_complete: false,
					diagnostics: [...result.diagnostics, warning],
				};
			}
			return { ...result, delivery_complete: true };
		});
	}

	async createBoardEditJob(
		appId: string,
		requestId: string | undefined,
		token: FlowIrCommitToken,
	): Promise<BoardEditJob> {
		return await invoke<BoardEditJob>("flowpilot_create_board_edit_job", {
			appId,
			requestId,
			token,
		});
	}

	async listBoardEditJobs(
		appId?: string,
		boardId?: string,
		includeTerminal = false,
	): Promise<BoardEditJob[]> {
		return await invoke<BoardEditJob[]>("flowpilot_list_board_edit_jobs", {
			appId,
			boardId,
			includeTerminal,
		});
	}

	async getBoardEditJob(jobId: string): Promise<BoardEditJob | undefined> {
		return (
			(await invoke<BoardEditJob | null>("flowpilot_get_board_edit_job", {
				jobId,
			})) ?? undefined
		);
	}

	async resolveBoardEditJob(
		jobId: string,
		approved: boolean,
		destructivePreapproved = false,
	): Promise<BoardEditJobResolution> {
		const job = await this.getBoardEditJob(jobId);
		const remoteIdentity =
			approved && job
				? await this.remoteBoardDeliveryIdentity(job.appId, job.boardId)
				: undefined;
		if (remoteIdentity && job?.remotePrincipalId) {
			// Bind the remote owner before the irreversible native mutation. Otherwise a
			// signed-out apply could create an exact outbox no future account may drain.
			const ownerMatch = evaluateCommandSyncRemoteIdentity(
				{
					remoteIdentityVersion: 1,
					remoteProfileId: job.remoteProfileId,
					remotePrincipalId: job.remotePrincipalId,
					remoteHub: job.remoteHub,
				},
				remoteIdentity,
			);
			if (!ownerMatch.apply) {
				throw new Error(
					`This FlowPilot review belongs to another remote identity: ${ownerMatch.refusalReason}.`,
				);
			}
		}
		const resolve = () =>
			invoke<BoardEditJobResolution>("flowpilot_resolve_board_edit_job", {
				jobId,
				approved,
				destructivePreapproved,
				remoteProfileId: remoteIdentity?.remoteProfileId,
				remotePrincipalId: remoteIdentity?.remotePrincipalId,
				remoteHub: remoteIdentity?.remoteHub,
			});
		if (!job) return await resolve();
		return await this.sequenceBoardMutation(job.appId, job.boardId, resolve);
	}

	async claimBoardEditJobDelivery(
		jobId: string,
	): Promise<BoardEditJobDeliveryClaim> {
		return await invoke<BoardEditJobDeliveryClaim>(
			"flowpilot_claim_board_edit_job_delivery",
			{ jobId },
		);
	}

	async ackBoardEditJobDelivery(
		jobId: string,
		deliveryLeaseId: string,
	): Promise<BoardEditJob> {
		return await invoke<BoardEditJob>("flowpilot_ack_board_edit_job_delivery", {
			jobId,
			deliveryLeaseId,
		});
	}

	/**
	 * Whether a run can be handed to the server for this app. `isOffline` also
	 * reports true when the app's visibility has never been cached, which is not
	 * a reason to give up on the server — only an app this device positively
	 * knows is local-only is.
	 */
	private async canReachServer(appId: string): Promise<boolean> {
		if (!this.backend.profile || !this.backend.auth) return false;
		return !(await this.backend.isLocalOnly(appId).catch(() => false));
	}

	async prerunBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IPrerunBoardResponse> {
		// Helper to build prerun response from local board
		const buildLocalPrerun = async (): Promise<IPrerunBoardResponse> => {
			const board = await this.fetchLocalBoard(appId, boardId, version);

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

		// Local-only apps have no server answer to ask for. An app whose
		// visibility is merely uncached is not one of them — treating it as one
		// leaves this preflight with only a board the device may not have.
		if (await this.backend.isLocalOnly(appId).catch(() => false)) {
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
