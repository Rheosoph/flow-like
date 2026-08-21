"use client";
import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";

import {
	type IApiKeyState,
	type IApiState,
	type IAppRouteState,
	type IAppState,
	IAppVisibility,
	type IBackendState,
	type IBit,
	type IBitState,
	type IBoardState,
	type ICapabilities,
	type IDatabaseState,
	type IEventState,
	type IGenericCommand,
	type IGraphState,
	type IHelperState,
	type IPageState,
	type IProfile,
	type IQueryState,
	type IRegistryState,
	type IRoleState,
	type ISalesState,
	type ISinkState,
	type IStorageState,
	type ITeamState,
	type ITemplateState,
	type IUsageState,
	type IUserState,
	type IWidgetState,
	LoadingScreen,
	type QueryClient,
	isAzureBlobStorageUrl,
	offlineSyncDB,
	useBackend,
	useBackendStore,
	useDownloadManager,
	useInvoke,
	useQueryClient,
} from "@flow-like/flow-like-ui";
import type {
	ICommandSync,
	ICommandSyncArchive,
} from "@flow-like/flow-like-ui/lib";
import type { IAIState } from "@flow-like/flow-like-ui/state/backend-state/ai-state";
import type { IAnalyticsState } from "@flow-like/flow-like-ui/state/backend-state/analytics-state";
import { createId } from "@paralleldrive/cuid2";
import Dexie, { type EntityTable } from "dexie";
import { useCallback, useEffect, useRef, useTransition } from "react";
import type { AuthContextProps } from "react-oidc-context";
import { appsDB } from "../lib/apps-db";
import { scheduleIDBCleanup } from "../lib/idb-maintenance";
import { isIOSDevice } from "../lib/platform";
import { type OnlineProfile, toLocalProfile } from "../lib/profile-sync";
import { AiState } from "./tauri-provider/ai-state";
import { AnalyticsState } from "./tauri-provider/analytics-state";
import { ApiKeyState } from "./tauri-provider/api-key-state";
import { TauriApiState } from "./tauri-provider/api-state";
import { AppState } from "./tauri-provider/app-state";
import { BitState } from "./tauri-provider/bit-state";
import { BoardState } from "./tauri-provider/board-state";
import type { CommandSyncRemoteIdentity } from "./tauri-provider/command-sync";
import { DatabaseState } from "./tauri-provider/db-state";
import { EventState } from "./tauri-provider/event-state";
import { GraphState } from "./tauri-provider/graph-state";
import { HelperState } from "./tauri-provider/helper-state";
import { PageState } from "./tauri-provider/page-state";
import { QueryState } from "./tauri-provider/query-state";
import { RegistryState } from "./tauri-provider/registry-state";
import { RoleState } from "./tauri-provider/role-state";
import { RouteState } from "./tauri-provider/route-state";
import { SalesState } from "./tauri-provider/sales-state";
import { SinkState } from "./tauri-provider/sink-state";
import { StorageState } from "./tauri-provider/storage-state";
import { TeamState } from "./tauri-provider/team-state";
import { TemplateState } from "./tauri-provider/template-state";
import { UsageState } from "./tauri-provider/usage-state";
import { UserState } from "./tauri-provider/user-state";
import { WidgetState } from "./tauri-provider/widget-state";

interface IBoardLineage {
	key: string;
	appId: string;
	boardId: string;
	/** Nanoseconds since epoch of the last remote updated_at applied or pushed past. */
	updatedAtNs: number;
	recordedAt: Date;
}

// Client-side sync lineage alongside the offline command queue: remembers the
// last remote board revision this client applied or successfully pushed past,
// so a stale remote snapshot (clock skew, equal timestamps, lagging replica)
// is refused instead of clobbering local state.
const boardLineageDB = new Dexie("BoardSyncLineage") as Dexie & {
	lineage: EntityTable<IBoardLineage, "key">;
};

boardLineageDB.version(1).stores({
	lineage: "key, [appId+boardId]",
});

const boardLineageKey = (appId: string, boardId: string): string =>
	`${appId}:${boardId}`;

// One-time resume guards for the whole app session
declare global {
	// eslint-disable-next-line no-var
	var __FL_DL_RESUME_PROMISE__: Promise<void> | undefined;
	// eslint-disable-next-line no-var
	var __FL_DL_RESUMED__: boolean | undefined;
}

export class TauriBackend implements IBackendState {
	appState: IAppState;
	apiState: IApiState;
	apiKeyState: IApiKeyState;
	bitState: IBitState;
	boardState: IBoardState;
	eventState: IEventState;
	helperState: IHelperState;
	roleState: IRoleState;
	routeState: IAppRouteState;
	storageState: IStorageState;
	teamState: ITeamState;
	templateState: ITemplateState;
	userState: IUserState;
	aiState: IAIState;
	dbState: IDatabaseState;
	graphState: IGraphState;
	queryState: IQueryState;
	widgetState: IWidgetState;
	pageState: IPageState;
	registryState: IRegistryState;
	sinkState: ISinkState;
	salesState: ISalesState;
	usageState: IUsageState;
	analyticsState: IAnalyticsState;

	private _apiState: TauriApiState;

	constructor(
		public readonly backgroundTaskHandler: (task: Promise<any>) => void,
		public queryClient?: QueryClient,
		public auth?: AuthContextProps,
		public profile?: IProfile,
		private readonly canHostMLX = false,
	) {
		this._apiState = new TauriApiState();
		this.apiState = this._apiState;
		this.apiKeyState = new ApiKeyState(this);
		this.appState = new AppState(this);
		this.bitState = new BitState(this);
		this.boardState = new BoardState(this);
		this.eventState = new EventState(this);
		this.helperState = new HelperState(this);
		this.roleState = new RoleState(this);
		this.routeState = new RouteState(this);
		this.storageState = new StorageState(this);
		this.teamState = new TeamState(this);
		this.templateState = new TemplateState(this);
		this.userState = new UserState(this);
		this.aiState = new AiState(this);
		this.dbState = new DatabaseState(this);
		this.graphState = new GraphState(this);
		this.queryState = new QueryState(this);
		this.widgetState = new WidgetState(this);
		this.pageState = new PageState(this);
		this.registryState = new RegistryState(this);
		this.sinkState = new SinkState();
		this.salesState = new SalesState(this);
		this.usageState = new UsageState(this);
		this.analyticsState = new AnalyticsState(this);
	}

	capabilities(): ICapabilities {
		const isIos = isIOSDevice();

		return {
			needsSignIn: isIos,
			canHostLlamaCPP: !isIos,
			canHostMLX: this.canHostMLX,
			canHostEmbeddings: true,
			canExecuteLocally: true,
		};
	}

	pushProfile(profile: IProfile) {
		this.profile = profile;
		this.refreshRemoteCatalogQueries();
	}

	pushAuthContext(auth: AuthContextProps) {
		this.auth = auth;
		this._apiState.setAuth(auth);
		const token = auth.user?.access_token ?? null;
		this.registryState
			.setAuthToken?.(token)
			?.catch((e) =>
				console.warn("[RegistryAuth] Failed to set auth token:", e),
			);
		this.refreshRemoteCatalogQueries();
	}

	pushQueryClient(queryClient: QueryClient) {
		this.queryClient = queryClient;
		this.refreshRemoteCatalogQueries();
	}

	private refreshRemoteCatalogQueries() {
		if (
			!this.queryClient ||
			!this.profile ||
			!this.auth ||
			this.auth.isLoading
		) {
			return;
		}

		// /use can mount while native auth/profile bootstrap is still in progress.
		// Its local-only result is otherwise cached under the force-refresh key and
		// remains fresh after the backend becomes capable of remote synchronization.
		void Promise.all([
			this.queryClient.invalidateQueries({
				queryKey: [this.routeState.getRoutes.name || "backendFn"],
				refetchType: "active",
			}),
			this.queryClient.invalidateQueries({
				queryKey: [this.eventState.getEvents.name || "backendFn"],
				refetchType: "active",
			}),
		]).catch((error) => {
			console.warn(
				"[CatalogSync] Failed to refresh routes/events after auth bootstrap:",
				error,
			);
		});
	}

	async isOffline(appId: string): Promise<boolean> {
		const visibility = await this.appVisibility(appId);
		return visibility === undefined || visibility === IAppVisibility.Offline;
	}

	/** True only when local metadata explicitly identifies an app as local-only. */
	async isLocalOnly(appId: string): Promise<boolean> {
		return (await this.appVisibility(appId)) === IAppVisibility.Offline;
	}

	private async appVisibility(
		appId: string,
	): Promise<IAppVisibility | undefined> {
		const status = await appsDB.visibility.get(appId);
		if (typeof status !== "undefined") {
			return status.visibility;
		}
		try {
			const app = await invoke<{ visibility?: IAppVisibility }>("get_app", {
				appId,
			});
			// Only a visibility the manifest actually states is worth caching.
			// Guessing "offline" here and persisting the guess is not a neutral
			// default: it is sticky, and it makes `isLocalOnly` claim a hosted app
			// is local-only, which closes the one door that downloads its boards
			// (`materializeBoardFromRemote`). An absent value stays unknown so the
			// callers that distinguish the two can still ask the hub.
			if (typeof app.visibility === "undefined") {
				return undefined;
			}
			await appsDB.visibility.put({ visibility: app.visibility, appId });
			return app.visibility;
		} catch {
			return undefined;
		}
	}

	async pushOfflineSyncCommand(
		appId: string,
		boardId: string,
		chunks: IGenericCommand[][],
		idempotencyKey?: string,
		chunkOffset = 0,
		commands?: IGenericCommand[],
		blockedReason?: string,
		remoteIdentity?: CommandSyncRemoteIdentity,
	) {
		console.log("Pushing offline sync command", {
			appId,
			boardId,
			chunkCount: chunks.length,
			commandCount:
				chunks.reduce((count, chunk) => count + chunk.length, 0) +
				(commands?.length ?? 0),
			blocked: Boolean(blockedReason),
		});
		const commandId = idempotencyKey
			? `${appId}${boardId}${idempotencyKey}`
			: createId();
		await offlineSyncDB.transaction("rw", offlineSyncDB.commands, async () => {
			// Preserve an already-queued undelivered tail. A renderer replay of the full receipt
			// must not overwrite it and resend chunks that the server already accepted.
			if (idempotencyKey && (await offlineSyncDB.commands.get(commandId)))
				return;
			const latest = await offlineSyncDB.commands.orderBy("sequence").last();
			const sequence = Math.max(Date.now() * 1000, (latest?.sequence ?? 0) + 1);
			await offlineSyncDB.commands.put({
				commandId,
				appId,
				boardId,
				chunks,
				commands,
				createdAt: new Date(),
				idempotencyKey,
				chunkOffset,
				sequence,
				blockedReason,
				remoteIdentityVersion: 1,
				remoteProfileId: remoteIdentity
					? remoteIdentity.remoteProfileId
					: this.profile?.id,
				remotePrincipalId: remoteIdentity
					? remoteIdentity.remotePrincipalId
					: this.auth?.user?.profile.sub,
				remoteHub: remoteIdentity
					? remoteIdentity.remoteHub
					: this.profile?.hub,
				deferReceiptAckUntilNativeTerminal:
					idempotencyKey?.startsWith("flowpilot-board-edit:") === true,
			});
		});
	}

	async checkpointOfflineSyncCommand(
		commandId: string,
		remainingChunks: IGenericCommand[][],
		chunkOffset: number,
		idempotencyKey: string,
		receiptKey: string,
		deferReceiptAck: boolean,
	): Promise<void> {
		await offlineSyncDB.transaction("rw", offlineSyncDB.commands, async () => {
			const existing = await offlineSyncDB.commands.get(commandId);
			if (!existing) return;
			const deferredReceiptAcks = deferReceiptAck
				? Array.from(
						new Set([...(existing.deferredReceiptAcks ?? []), receiptKey]),
					)
				: existing.deferredReceiptAcks;
			await offlineSyncDB.commands.put({
				...existing,
				commands: undefined,
				chunks: remainingChunks,
				chunkOffset,
				idempotencyKey,
				pendingReceiptAck: deferReceiptAck ? undefined : receiptKey,
				deferredReceiptAcks,
			});
		});
	}

	async migrateLegacyOfflineSyncCommand(
		commandId: string,
		chunks: IGenericCommand[][],
		idempotencyKey: string,
	): Promise<void> {
		await offlineSyncDB.transaction("rw", offlineSyncDB.commands, async () => {
			const existing = await offlineSyncDB.commands.get(commandId);
			if (!existing || existing.chunks) return;
			// Freeze the exact recovery partition and key before its first POST. Recomputing
			// either after a crash could reuse `:0` for a different digest and permanently
			// conflict with the server's durable idempotency receipt.
			await offlineSyncDB.commands.put({
				...existing,
				commands: undefined,
				chunks,
				chunkOffset: existing.chunkOffset ?? 0,
				idempotencyKey,
			});
		});
	}

	async blockOfflineSyncCommand(
		commandId: string,
		blockedReason: string,
	): Promise<void> {
		await offlineSyncDB.transaction("rw", offlineSyncDB.commands, async () => {
			const existing = await offlineSyncDB.commands.get(commandId);
			if (!existing) return;
			await offlineSyncDB.commands.put({
				...existing,
				blockedReason,
			});
		});
	}

	/**
	 * Replace the payload of a queued batch that has never been delivered.
	 *
	 * Only used to restate `on_update`-derived node state a batch failed to capture, so the batch
	 * becomes replayable on the Hub. The row keeps its id, sequence, idempotency key and remote
	 * identity, so ordering and ownership are untouched; the caller must guarantee the server has
	 * not seen this payload, because the durable receipt is keyed on its digest.
	 */
	async repairOfflineSyncCommand(
		commandId: string,
		chunks: IGenericCommand[][],
	): Promise<void> {
		await offlineSyncDB.transaction("rw", offlineSyncDB.commands, async () => {
			const existing = await offlineSyncDB.commands.get(commandId);
			if (!existing) return;
			if ((existing.chunkOffset ?? 0) !== 0 || existing.pendingReceiptAck) {
				return;
			}
			await offlineSyncDB.commands.put({
				...existing,
				chunks,
				commands: undefined,
			});
		});
	}

	/**
	 * Record why a delivery attempt failed. Purely diagnostic: the batch stays queued and ordered,
	 * but the reason survives restarts so a permanently rejected payload is visible instead of
	 * silently blocking every later edit for the same board.
	 */
	async recordOfflineSyncFailure(
		commandId: string,
		failure: { status?: number; message: string },
	): Promise<void> {
		await offlineSyncDB.transaction("rw", offlineSyncDB.commands, async () => {
			const existing = await offlineSyncDB.commands.get(commandId);
			if (!existing) return;
			await offlineSyncDB.commands.put({
				...existing,
				lastFailureStatus: failure.status,
				lastFailureMessage: failure.message.slice(0, 2000),
				lastFailureAt: new Date(),
				failedAttempts: (existing.failedAttempts ?? 0) + 1,
			});
		});
	}

	async bindLegacyOfflineSyncCommand(
		commandId: string,
		remoteIdentity: CommandSyncRemoteIdentity,
	): Promise<void> {
		await offlineSyncDB.transaction("rw", offlineSyncDB.commands, async () => {
			const existing = await offlineSyncDB.commands.get(commandId);
			if (!existing || existing.remoteIdentityVersion === 1) return;
			await offlineSyncDB.commands.put({
				...existing,
				remoteIdentityVersion: 1,
				remoteProfileId: remoteIdentity.remoteProfileId,
				remotePrincipalId: remoteIdentity.remotePrincipalId,
				remoteHub: remoteIdentity.remoteHub,
			});
		});
	}

	async completeOfflineSyncReceiptAck(
		commandId: string,
		pendingReceiptAck: string,
	): Promise<void> {
		await offlineSyncDB.transaction("rw", offlineSyncDB.commands, async () => {
			const existing = await offlineSyncDB.commands.get(commandId);
			if (!existing || existing.pendingReceiptAck !== pendingReceiptAck) return;
			if (
				(existing.chunks?.length ?? 0) === 0 &&
				(existing.deferredReceiptAcks?.length ?? 0) === 0
			) {
				await offlineSyncDB.commands.delete(commandId);
				return;
			}
			await offlineSyncDB.commands.put({
				...existing,
				pendingReceiptAck: undefined,
			});
		});
	}

	async deferOfflineSyncReceiptAck(
		commandId: string,
		receiptKey: string,
	): Promise<void> {
		await offlineSyncDB.transaction("rw", offlineSyncDB.commands, async () => {
			const existing = await offlineSyncDB.commands.get(commandId);
			if (!existing || existing.pendingReceiptAck !== receiptKey) return;
			await offlineSyncDB.commands.put({
				...existing,
				pendingReceiptAck: undefined,
				deferReceiptAckUntilNativeTerminal: true,
				deferredReceiptAcks: Array.from(
					new Set([...(existing.deferredReceiptAcks ?? []), receiptKey]),
				),
			});
		});
	}

	async getOfflineSyncCommands(
		appId: string,
		boardId: string,
	): Promise<ICommandSync[]> {
		const commands = await offlineSyncDB.commands
			.where({
				appId: appId,
				boardId: boardId,
			})
			.toArray();

		return commands.toSorted((a, b) => {
			const aOrder = a.sequence ?? a.createdAt.getTime() * 1000;
			const bOrder = b.sequence ?? b.createdAt.getTime() * 1000;
			return aOrder - bOrder || a.commandId.localeCompare(b.commandId);
		});
	}

	async clearOfflineSyncCommands(
		commandId: string,
		appId: string,
		boardId: string,
	): Promise<void> {
		await offlineSyncDB.commands.delete(commandId);
	}

	/**
	 * Move queued batches out of the outbox as part of an explicit server-authoritative reset.
	 *
	 * Archive and delete share one transaction: a crash between them would either lose the only
	 * copy of an undelivered edit or leave it queued and still wedging the board.
	 */
	async archiveOfflineSyncCommands(
		appId: string,
		boardId: string,
		commandIds: readonly string[],
		archiveReason: string,
	): Promise<number> {
		if (commandIds.length === 0) return 0;
		const archivedAt = new Date();
		let archived = 0;
		await offlineSyncDB.transaction(
			"rw",
			offlineSyncDB.commands,
			offlineSyncDB.discarded,
			async () => {
				for (const commandId of commandIds) {
					const existing = await offlineSyncDB.commands.get(commandId);
					if (
						!existing ||
						existing.appId !== appId ||
						existing.boardId !== boardId
					)
						continue;
					await offlineSyncDB.discarded.put({
						...existing,
						archiveId: `${commandId}${archivedAt.getTime()}`,
						archivedAt,
						archiveReason,
					});
					await offlineSyncDB.commands.delete(commandId);
					archived += 1;
				}
			},
		);
		return archived;
	}

	async listOfflineSyncArchive(
		appId: string,
		boardId: string,
	): Promise<ICommandSyncArchive[]> {
		const entries = await offlineSyncDB.discarded
			.where({ appId: appId, boardId: boardId })
			.toArray();
		return entries.toSorted(
			(a, b) => a.archivedAt.getTime() - b.archivedAt.getTime(),
		);
	}

	async getBoardLineage(
		appId: string,
		boardId: string,
	): Promise<number | undefined> {
		const entry = await boardLineageDB.lineage.get(
			boardLineageKey(appId, boardId),
		);
		return entry?.updatedAtNs;
	}

	/** Max-merge: the lineage timestamp only ever moves forward. */
	async recordBoardLineage(
		appId: string,
		boardId: string,
		updatedAtNs: number,
	): Promise<void> {
		if (!Number.isFinite(updatedAtNs) || updatedAtNs <= 0) return;

		const key = boardLineageKey(appId, boardId);
		await boardLineageDB.transaction("rw", boardLineageDB.lineage, async () => {
			const existing = await boardLineageDB.lineage.get(key);
			if (existing && existing.updatedAtNs >= updatedAtNs) return;
			await boardLineageDB.lineage.put({
				key,
				appId,
				boardId,
				updatedAtNs,
				recordedAt: new Date(),
			});
		});
	}

	/**
	 * Forget the last applied revision for one board.
	 *
	 * `recordBoardLineage` only moves forward, so a lineage stamped from a clock-skewed local
	 * board refuses every real server revision afterwards. A server-authoritative reset must
	 * clear it, or the snapshot it just fetched is rejected by the next background sync.
	 */
	async clearBoardLineage(appId: string, boardId: string): Promise<void> {
		await boardLineageDB.lineage.delete(boardLineageKey(appId, boardId));
	}

	async uploadSignedUrl(
		signedUrl: string,
		file: File,
		completedFiles: number,
		totalFiles: number,
		onProgress?: (progress: number) => void,
	): Promise<void> {
		await new Promise<void>((resolve, reject) => {
			const xhr = new XMLHttpRequest();

			xhr.upload.addEventListener("progress", (event) => {
				if (event.lengthComputable) {
					const fileProgress = event.loaded / event.total;
					const totalProgress =
						((completedFiles + fileProgress) / totalFiles) * 100;
					onProgress?.(totalProgress);
				}
			});

			xhr.addEventListener("load", () => {
				if (xhr.status >= 200 && xhr.status < 300) {
					resolve();
				} else {
					reject(
						new Error(
							i18next.t(
								"uploadFailedWithStatusStatusStatustext",
								"Upload failed with status {{status}}: {{statusText}}",
								{ status: xhr.status, statusText: xhr.statusText },
							),
						),
					);
				}
			});

			xhr.addEventListener("error", () => {
				reject(new Error("Upload failed: Network error (possible CORS issue)"));
			});

			xhr.open("PUT", signedUrl);
			xhr.setRequestHeader(
				"Content-Type",
				file.type || "application/octet-stream",
			);

			// Azure Blob Storage requires x-ms-blob-type header
			if (isAzureBlobStorageUrl(signedUrl)) {
				xhr.setRequestHeader("x-ms-blob-type", "BlockBlob");
			}

			xhr.send(file);
		});

		onProgress?.((completedFiles / totalFiles) * 100);
	}
}

export function TauriProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const queryClient = useQueryClient();
	const { backend, setBackend } = useBackendStore();
	const { setDownloadBackend, download } = useDownloadManager();
	const [isPending, startTransition] = useTransition();

	// Safety to avoid state updates after unmount during resume
	const mountedRef = useRef(true);
	useEffect(() => {
		mountedRef.current = true;
		return () => {
			mountedRef.current = false;
		};
	}, []);

	// Resume downloads exactly once per app lifecycle
	const resumeDownloads = useCallback(async () => {
		if (globalThis.__FL_DL_RESUME_PROMISE__) {
			await globalThis.__FL_DL_RESUME_PROMISE__;
			return;
		}

		globalThis.__FL_DL_RESUME_PROMISE__ = (async () => {
			try {
				// Small defer to let backend wire up
				await new Promise((r) => setTimeout(r, 100));

				// First-time resume: ask backend for items to resume
				if (!globalThis.__FL_DL_RESUMED__) {
					console.time("Resuming Downloads (init_downloads)");
					const downloads = await invoke<{ [key: string]: IBit }>(
						"init_downloads",
					);
					console.timeEnd("Resuming Downloads (init_downloads)");
					const items = Object.keys(downloads).map((bitId) => downloads[bitId]);

					if (items.length) {
						console.time("Resuming download requests");
						await Promise.allSettled(items.map((item) => download(item)));
						console.timeEnd("Resuming download requests");
					}
					globalThis.__FL_DL_RESUMED__ = true;
					return;
				}

				// Subsequent mounts: only hydrate UI state, do not re-trigger init
				console.time("Hydrating Downloads (get_downloads)");
				const snapshot = await invoke<{ [key: string]: IBit }>("get_downloads");
				console.timeEnd("Hydrating Downloads (get_downloads)");
				const items = Object.keys(snapshot).map((bitId) => snapshot[bitId]);

				// Calling download(item) is safe because the manager de-duplicates in-flight calls.
				if (items.length) {
					await Promise.allSettled(items.map((item) => download(item)));
				}
			} catch (e) {
				console.error("resumeDownloads failed:", e);
			}
		})();

		await globalThis.__FL_DL_RESUME_PROMISE__;
	}, [download]);

	// Kick off resume when a backend is set
	useEffect(() => {
		if (!backend) return;
		startTransition(() => {
			void resumeDownloads();
		});
	}, [backend, resumeDownloads]);

	// Keep backend references in sync
	useEffect(() => {
		if (backend && backend instanceof TauriBackend && queryClient) {
			backend.pushQueryClient(queryClient);
		}
	}, [backend, queryClient]);

	// Registry changes can also change dynamic pins on already-open boards.
	// Native refreshes those cached boards before emitting this event; invalidate
	// both query families so the editor picks up the refreshed contracts.
	useEffect(() => {
		const unlisten = listen("catalog-updated", () => {
			queryClient.invalidateQueries({ queryKey: ["getCatalog"] });
			queryClient.invalidateQueries({ queryKey: ["app-catalog-nodes"] });
			queryClient.invalidateQueries({ queryKey: ["getBoard"] });
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, [queryClient]);

	// Eagerly initialize the WASM package registry so installed packages
	// appear in the catalog without requiring the user to visit the store.
	useEffect(() => {
		if (!backend) return;
		backend.registryState
			.init?.()
			.then(() => {
				queryClient.invalidateQueries({ queryKey: ["getCatalog"] });
				queryClient.invalidateQueries({ queryKey: ["app-catalog-nodes"] });
				// The event listener may still be attaching during startup.
				queryClient.invalidateQueries({ queryKey: ["getBoard"] });
			})
			.catch((e: unknown) => {
				console.warn("Registry init (eager):", e);
			});
	}, [backend, queryClient]);

	useEffect(() => {
		let cancelled = false;

		const initializeBackend = async () => {
			let canHostMLX = false;
			try {
				canHostMLX = await invoke<boolean>("can_host_mlx");
			} catch (error) {
				console.warn("Failed to detect MLX support:", error);
			}

			// Publish the backend only after capability detection. Consumers never
			// observe a stale backend object whose synchronous capabilities changed
			// without a React store update.
			if (cancelled || !mountedRef.current) return;

			console.time("TauriProvider Initialization");
			const backend = new TauriBackend(
				(promise) => {
					promise
						.then((result) => {
							console.log("Background task completed:", result);
						})
						.catch((error) => {
							console.error("Background task failed:", error);
						});
				},
				queryClient,
				undefined,
				undefined,
				canHostMLX,
			);
			console.timeEnd("TauriProvider Initialization");

			console.time("Setting Backend");
			setBackend(backend);
			console.timeEnd("Setting Backend");

			console.time("Setting Download Backend");
			setDownloadBackend(backend);
			console.timeEnd("Setting Download Backend");

			scheduleIDBCleanup();
		};

		void initializeBackend();
		return () => {
			cancelled = true;
		};
	}, [queryClient, setBackend, setDownloadBackend]);

	if (!backend) {
		return <LoadingScreen progress={50} />;
	}

	return <>{children}</>;
}

export function ProfileSyncer({
	auth,
}: { auth: { isAuthenticated: boolean; accessToken?: string } }) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		true,
	);

	const isAuthenticated =
		backend instanceof TauriBackend && auth.isAuthenticated;
	const accessToken = auth.accessToken;
	const hubUrl = profile.data?.hub;

	const syncingRef = useRef(false);
	const lastSyncAtRef = useRef<number>(0);
	const MIN_SYNC_INTERVAL_MS = 60_000;

	useEffect(() => {
		if (profile.data && backend instanceof TauriBackend) {
			backend.pushProfile(profile.data);
		}
	}, [profile.data, backend]);

	useEffect(() => {
		if (!(backend instanceof TauriBackend)) {
			console.log("[ProfileSync] Skipping: not TauriBackend");
			return;
		}
		if (!isAuthenticated || !accessToken) {
			console.log("[ProfileSync] Skipping: missing auth state", {
				isAuthenticated,
				hasAccessToken: !!accessToken,
			});
			return;
		}

		const isHttpPath = (path?: string | null): boolean => {
			if (!path) return false;
			return path.startsWith("http://") || path.startsWith("https://");
		};

		const isAssetProxyPath = (path?: string | null): boolean => {
			if (!path) return false;
			return (
				path.startsWith("asset://") ||
				path.startsWith("http://asset.localhost/") ||
				path.startsWith("https://asset.localhost/")
			);
		};

		const isLocalFilePath = (path?: string | null): boolean => {
			if (!path) return false;
			if (isAssetProxyPath(path)) return true;
			if (isHttpPath(path)) return false;
			return true;
		};

		const shouldReplaceWithServerImage = (
			localPath?: string | null,
		): boolean => {
			if (!localPath) return true;
			if (isAssetProxyPath(localPath)) return true;
			if (isHttpPath(localPath)) return true;
			return false;
		};

		const getExtension = (path: string): string => {
			const ext = path.split(".").pop()?.toLowerCase() ?? "png";
			if (ext === "jpeg") return "jpg";
			return ext;
		};

		const getContentType = (ext: string): string => {
			switch (ext) {
				case "webp":
					return "image/webp";
				case "jpg":
				case "jpeg":
					return "image/jpeg";
				case "gif":
					return "image/gif";
				case "svg":
					return "image/svg+xml";
				default:
					return "image/png";
			}
		};

		const uploadIconByProfileId = async (
			profileId: string,
			iconField: "icon" | "thumbnail",
			signedUrl: string,
		): Promise<boolean> => {
			try {
				const iconPath = await invoke<string | null>("get_profile_icon_path", {
					profileId,
					field: iconField,
				});
				if (!iconPath) return false;

				const fileData = await invoke<number[]>("read_profile_icon", {
					iconPath,
				});
				const ext = getExtension(iconPath);
				const bytes = new Uint8Array(fileData);

				const uploadResponse = await tauriFetch(signedUrl, {
					method: "PUT",
					headers: { "Content-Type": getContentType(ext) },
					body: bytes,
				});
				return uploadResponse.ok;
			} catch (error) {
				console.warn(`Failed to upload ${iconField} for ${profileId}:`, error);
				return false;
			}
		};

		const requestProfileMediaUploadUrls = async (
			profileId: string,
			apiBase: string,
			iconExt?: string,
			thumbnailExt?: string,
		): Promise<{
			icon_upload_url?: string | null;
			thumbnail_upload_url?: string | null;
		} | null> => {
			if (!iconExt && !thumbnailExt) {
				return null;
			}

			try {
				const response = await tauriFetch(
					`${apiBase}/api/v1/profile/${encodeURIComponent(profileId)}`,
					{
						method: "POST",
						headers: {
							"Content-Type": "application/json",
							Authorization: `Bearer ${accessToken}`,
						},
						body: JSON.stringify({
							icon_upload_ext: iconExt,
							thumbnail_upload_ext: thumbnailExt,
						}),
					},
				);

				if (!response.ok) {
					const errorBody = await response.text().catch(() => "<no body>");
					console.error(
						"[ProfileSync] Failed to request fallback media upload URLs:",
						profileId,
						response.status,
						errorBody,
					);
					return null;
				}

				const result = (await response.json()) as {
					icon_upload_url?: string | null;
					thumbnail_upload_url?: string | null;
				};
				return result;
			} catch (error) {
				console.error(
					"[ProfileSync] Error requesting fallback media upload URLs:",
					profileId,
					error,
				);
				return null;
			}
		};

		/**
		 * Delete a profile locally, handling the case where it may be the current profile.
		 * Switches to another profile first if needed.
		 * Returns true if the deleted profile was the current profile.
		 */
		const deleteProfileLocally = async (
			profileId: string,
		): Promise<boolean> => {
			const currentId = await invoke<string>("get_current_profile_id").catch(
				() => null,
			);
			let wasCurrentProfile = false;

			if (currentId === profileId) {
				wasCurrentProfile = true;
				const allProfiles =
					await invoke<Record<string, { hub_profile: IProfile }>>(
						"get_profiles_raw",
					);
				const otherId = allProfiles
					? Object.keys(allProfiles).find((id) => id !== profileId)
					: undefined;
				if (otherId) {
					await invoke("set_current_profile", { profileId: otherId });
				}
			}

			await invoke("delete_profile", { profileId });
			await appsDB.shortcuts.where("profileId").equals(profileId).delete();
			return wasCurrentProfile;
		};

		const syncProfiles = async () => {
			if (syncingRef.current) {
				console.log("[ProfileSync] Already syncing, skipping");
				return;
			}
			const now = Date.now();
			if (now - lastSyncAtRef.current < MIN_SYNC_INTERVAL_MS) {
				console.log("[ProfileSync] Cooldown active, skipping");
				return;
			}
			syncingRef.current = true;
			lastSyncAtRef.current = now;

			try {
				console.log("[ProfileSync] Starting profile sync...");

				const baseUrl =
					process.env.NEXT_PUBLIC_API_URL ?? hubUrl ?? "api.flow-like.com";
				const protocol = profile.data?.secure === false ? "http" : "https";
				const apiBase = (
					baseUrl.startsWith("http") ? baseUrl : `${protocol}://${baseUrl}`
				).replace(/\/+$/, "");
				console.log(
					"[ProfileSync] API base:",
					apiBase,
					"hubUrl:",
					hubUrl,
					"secure:",
					profile.data?.secure,
				);

				let rawProfiles =
					await invoke<Record<string, { hub_profile: IProfile }>>(
						"get_profiles_raw",
					);
				console.log(
					"[ProfileSync] Raw profiles from Tauri:",
					rawProfiles ? Object.keys(rawProfiles) : "null",
				);

				// If no local profiles exist, pull from server first (new device scenario)
				if (!rawProfiles || Object.keys(rawProfiles).length === 0) {
					console.log(
						"[ProfileSync] No local profiles — pulling from server first...",
					);
					try {
						const pullResponse = await tauriFetch(`${apiBase}/api/v1/profile`, {
							method: "GET",
							headers: {
								"Content-Type": "application/json",
								Authorization: `Bearer ${accessToken}`,
							},
						});

						if (pullResponse.ok) {
							const allServerProfiles =
								(await pullResponse.json()) as OnlineProfile[];
							const serverProfiles = allServerProfiles.filter(
								(p) => !p.deleted_at,
							);

							if (serverProfiles.length > 0) {
								console.log(
									"[ProfileSync] Found",
									serverProfiles.length,
									t(
										"profilesOnServerCreatingLocally",
										"profiles on server, creating locally...",
									),
								);
								let firstProfileId: string | null = null;

								for (const onlineProfile of serverProfiles) {
									try {
										await invoke("upsert_profile", {
											profile: toLocalProfile(onlineProfile),
										});
										if (!firstProfileId) firstProfileId = onlineProfile.id;
									} catch (error) {
										console.error(
											"[ProfileSync] Failed to create profile from server:",
											onlineProfile.id,
											error,
										);
									}

									if (onlineProfile.shortcuts) {
										for (const shortcut of onlineProfile.shortcuts) {
											await appsDB.shortcuts.put(shortcut);
										}
									}
								}

								if (firstProfileId) {
									try {
										await invoke("set_current_profile", {
											profileId: firstProfileId,
										});
									} catch (error) {
										console.error(
											"[ProfileSync] Failed to set current profile:",
											error,
										);
									}
								}

								// Re-fetch local profiles after creating from server
								rawProfiles =
									await invoke<Record<string, { hub_profile: IProfile }>>(
										"get_profiles_raw",
									);
								console.log(
									"[ProfileSync] After server pull, local profiles:",
									rawProfiles ? Object.keys(rawProfiles) : "null",
								);
							}
						}
					} catch (error) {
						console.warn(
							"[ProfileSync] Failed to pull server profiles on fresh device:",
							error,
						);
					}

					// If still no profiles after server pull, nothing to sync
					if (!rawProfiles || Object.keys(rawProfiles).length === 0) {
						console.log(
							"[ProfileSync] Still no profiles after server pull, aborting",
						);
						return;
					}
				}

				const visibilityRecords = await appsDB.visibility.toArray();
				const offlineAppIds = new Set(
					visibilityRecords
						.filter((v) => v.visibility === IAppVisibility.Offline)
						.map((v) => v.appId),
				);

				const profileShortcuts: Map<string, any[]> = new Map();
				for (const p of Object.values(rawProfiles)) {
					const profileId = p.hub_profile.id;
					if (profileId) {
						const shortcuts = await appsDB.shortcuts
							.where("profileId")
							.equals(profileId)
							.toArray();
						profileShortcuts.set(profileId, shortcuts);
					}
				}

				const profilesWithLocalImages: Map<
					string,
					{ icon: boolean; thumbnail: boolean }
				> = new Map();
				const profileLocalImageExts: Map<
					string,
					{ icon?: string; thumbnail?: string }
				> = new Map();

				const profilesToSync = Object.values(rawProfiles).map((p) => {
					const hubProfile = p.hub_profile;
					const filteredApps = hubProfile.apps?.filter(
						(app) => !offlineAppIds.has(app.app_id),
					);

					const hasLocalIcon = isLocalFilePath(hubProfile.icon);
					const hasLocalThumbnail = isLocalFilePath(hubProfile.thumbnail);
					const iconExt =
						hasLocalIcon && hubProfile.icon
							? getExtension(hubProfile.icon)
							: undefined;
					const thumbnailExt =
						hasLocalThumbnail && hubProfile.thumbnail
							? getExtension(hubProfile.thumbnail)
							: undefined;

					if ((hasLocalIcon || hasLocalThumbnail) && hubProfile.id) {
						profilesWithLocalImages.set(hubProfile.id, {
							icon: hasLocalIcon,
							thumbnail: hasLocalThumbnail,
						});
						profileLocalImageExts.set(hubProfile.id, {
							icon: iconExt,
							thumbnail: thumbnailExt,
						});
					}

					const shortcuts = hubProfile.id
						? profileShortcuts.get(hubProfile.id)
						: undefined;

					const updatedAt = hubProfile.updated
						? new Date(hubProfile.updated).toISOString()
						: undefined;
					const createdAt = hubProfile.created
						? new Date(hubProfile.created).toISOString()
						: undefined;

					return {
						id: hubProfile.id,
						name: hubProfile.name,
						description: hubProfile.description,
						icon_upload_ext: iconExt,
						thumbnail_upload_ext: thumbnailExt,
						interests: hubProfile.interests,
						tags: hubProfile.tags,
						theme: hubProfile.theme,
						bit_ids: hubProfile.bits,
						apps: filteredApps,
						shortcuts: shortcuts,
						hubs: hubProfile.hubs,
						settings: hubProfile.settings,
						createdAt,
						updatedAt,
					};
				});

				if (profilesToSync.length === 0) {
					console.log("[ProfileSync] No profiles to sync after filtering");
					return;
				}

				console.log(
					"[ProfileSync] Sending",
					profilesToSync.length,
					t("profilesToSync", "profiles to sync:"),
					JSON.stringify(profilesToSync, null, 2),
				);

				const response = await tauriFetch(`${apiBase}/api/v1/profile/sync`, {
					method: "POST",
					headers: {
						"Content-Type": "application/json",
						Authorization: `Bearer ${accessToken}`,
					},
					body: JSON.stringify(profilesToSync),
				});

				type SyncResult = {
					synced: string[];
					created: Array<{
						local_id: string;
						server_id: string;
						icon_upload_url?: string;
						thumbnail_upload_url?: string;
					}>;
					updated: Array<{
						id: string;
						icon_upload_url?: string;
						thumbnail_upload_url?: string;
					}>;
					skipped: string[];
					deleted: string[];
				};

				let result: SyncResult = {
					synced: [],
					created: [],
					updated: [],
					skipped: [],
					deleted: [],
				};

				if (!response.ok) {
					const errorBody = await response.text().catch(() => "<no body>");
					console.error(
						"[ProfileSync] Sync request failed:",
						response.status,
						response.statusText,
						errorBody,
					);
					return;
				} else {
					result = (await response.json()) as SyncResult;
					console.log(
						"[ProfileSync] Sync result:",
						JSON.stringify(result, null, 2),
					);
				}

				// Delete local profiles that the server reports as soft-deleted (tombstones)
				for (const deletedId of result.deleted) {
					console.log(
						"[ProfileSync] Server reports profile as deleted (tombstone):",
						deletedId,
					);
					try {
						await deleteProfileLocally(deletedId);
					} catch (error) {
						console.error(
							"[ProfileSync] Failed to delete tombstoned profile locally:",
							deletedId,
							error,
						);
					}
				}

				for (const created of result.created) {
					console.log(
						"[ProfileSync] Processing created profile:",
						created.local_id,
						"->",
						created.server_id,
					);
					const localImages = profilesWithLocalImages.get(created.local_id);
					if (localImages?.icon && created.icon_upload_url) {
						await uploadIconByProfileId(
							created.local_id,
							"icon",
							created.icon_upload_url,
						);
					}
					if (localImages?.thumbnail && created.thumbnail_upload_url) {
						await uploadIconByProfileId(
							created.local_id,
							"thumbnail",
							created.thumbnail_upload_url,
						);
					}
				}

				for (const updated of result.updated) {
					const localImages = profilesWithLocalImages.get(updated.id);
					if (localImages?.icon && updated.icon_upload_url) {
						await uploadIconByProfileId(
							updated.id,
							"icon",
							updated.icon_upload_url,
						);
					}
					if (localImages?.thumbnail && updated.thumbnail_upload_url) {
						await uploadIconByProfileId(
							updated.id,
							"thumbnail",
							updated.thumbnail_upload_url,
						);
					}
				}

				for (const { local_id, server_id } of result.created) {
					console.log(
						"[ProfileSync] Remapping profile ID:",
						local_id,
						"->",
						server_id,
					);
					await invoke("remap_profile_id", {
						localId: local_id,
						serverId: server_id,
					});
					const shortcuts = await appsDB.shortcuts
						.where("profileId")
						.equals(local_id)
						.toArray();
					for (const shortcut of shortcuts) {
						await appsDB.shortcuts.update(shortcut.id, {
							profileId: server_id,
						});
					}
				}

				// Phase 6: Pull remote state
				console.log(
					"[ProfileSync] Phase 6: Pulling remote profiles from",
					`${apiBase}/api/v1/profile`,
				);
				try {
					const profilesResponse = await tauriFetch(
						`${apiBase}/api/v1/profile`,
						{
							method: "GET",
							headers: {
								"Content-Type": "application/json",
								Authorization: `Bearer ${accessToken}`,
							},
						},
					);

					if (!profilesResponse.ok) {
						const pullErrorBody = await profilesResponse
							.text()
							.catch(() => "<no body>");
						console.error(
							"[ProfileSync] Pull profiles failed:",
							profilesResponse.status,
							pullErrorBody,
						);
						return;
					}

					const allOnlineProfiles =
						(await profilesResponse.json()) as OnlineProfile[];
					const tombstoneIds = new Set(
						allOnlineProfiles.filter((p) => p.deleted_at).map((p) => p.id),
					);
					const onlineProfiles = allOnlineProfiles.filter((p) => !p.deleted_at);
					const onlineProfilesById = new Map(
						onlineProfiles.map((p) => [p.id, p]),
					);

					const onlineProfileIds = new Set(onlineProfiles.map((p) => p.id));

					// Fallback media sync path:
					// if the bulk sync endpoint returns no upload URLs, backfill media for profiles
					// that still have local files but missing media on the server.
					for (const [profileId, localImages] of profilesWithLocalImages) {
						const remoteProfile = onlineProfilesById.get(profileId);
						if (!remoteProfile) continue;

						const localExts = profileLocalImageExts.get(profileId);
						const needsIconUpload =
							localImages.icon && !remoteProfile.icon && !!localExts?.icon;
						const needsThumbnailUpload =
							localImages.thumbnail &&
							!remoteProfile.thumbnail &&
							!!localExts?.thumbnail;

						if (!needsIconUpload && !needsThumbnailUpload) continue;

						console.log(
							"[ProfileSync] Fallback media upload requested for profile:",
							profileId,
							"needsIconUpload:",
							needsIconUpload,
							"needsThumbnailUpload:",
							needsThumbnailUpload,
						);

						const fallbackUrls = await requestProfileMediaUploadUrls(
							profileId,
							apiBase,
							needsIconUpload ? localExts?.icon : undefined,
							needsThumbnailUpload ? localExts?.thumbnail : undefined,
						);
						if (!fallbackUrls) continue;

						if (needsIconUpload && fallbackUrls.icon_upload_url) {
							await uploadIconByProfileId(
								profileId,
								"icon",
								fallbackUrls.icon_upload_url,
							);
						}
						if (needsThumbnailUpload && fallbackUrls.thumbnail_upload_url) {
							await uploadIconByProfileId(
								profileId,
								"thumbnail",
								fallbackUrls.thumbnail_upload_url,
							);
						}
					}

					const currentLocalProfiles =
						await invoke<Record<string, { hub_profile: IProfile }>>(
							"get_profiles_raw",
						);
					let deletedCurrentProfile = false;

					if (currentLocalProfiles) {
						for (const localProfileId of Object.keys(currentLocalProfiles)) {
							const localProfile = currentLocalProfiles[localProfileId];

							// Delete if the server reports this profile as tombstoned
							const isTombstoned = tombstoneIds.has(localProfileId);
							// Delete if this profile has online apps but no longer exists on the server
							const hasOnlineApps = localProfile.hub_profile.apps?.some(
								(app) => !offlineAppIds.has(app.app_id),
							);
							const isStale =
								hasOnlineApps && !onlineProfileIds.has(localProfileId);

							if (isTombstoned || isStale) {
								console.log(
									"[ProfileSync] Deleting local profile:",
									localProfileId,
									isTombstoned ? "(tombstoned)" : "(stale)",
								);
								try {
									const wasCurrent = await deleteProfileLocally(localProfileId);
									if (wasCurrent) deletedCurrentProfile = true;
								} catch (error) {
									console.error(
										"[ProfileSync] Failed to delete local profile:",
										localProfileId,
										error,
									);
								}
							}
						}
					}

					const remainingProfiles =
						await invoke<Record<string, { hub_profile: IProfile }>>(
							"get_profiles_raw",
						);

					if (
						(!remainingProfiles ||
							Object.keys(remainingProfiles).length === 0) &&
						onlineProfiles.length === 0
					) {
						if (typeof window !== "undefined") {
							window.location.href = "/onboarding";
						}
						return;
					}

					if (
						deletedCurrentProfile &&
						remainingProfiles &&
						Object.keys(remainingProfiles).length > 0
					) {
						const firstProfileId = Object.keys(remainingProfiles)[0];
						try {
							await invoke("set_current_profile", {
								profileId: firstProfileId,
							});
						} catch (error) {
							console.error("Failed to switch profile:", error);
						}
					}

					// Create or merge profiles from server
					const latestLocal =
						await invoke<
							Record<
								string,
								{
									hub_profile: IProfile;
									execution_settings: any;
									updated: string;
									created: string;
								}
							>
						>("get_profiles_raw");

					for (const onlineProfile of onlineProfiles) {
						const localProfile = latestLocal?.[onlineProfile.id];

						if (!localProfile) {
							console.log(
								"[ProfileSync] Creating local profile from remote:",
								onlineProfile.id,
								onlineProfile.name,
							);
							try {
								await invoke("upsert_profile", {
									profile: toLocalProfile(onlineProfile),
								});
							} catch (error) {
								console.error(
									"[ProfileSync] Failed to create local profile:",
									onlineProfile.id,
									error,
								);
							}
						} else {
							// Merge: update existing local profile if server is newer
							const serverTime = new Date(onlineProfile.updated_at).getTime();
							const localTime = new Date(
								localProfile.hub_profile.updated || localProfile.updated || 0,
							).getTime();

							if (serverTime > localTime) {
								console.log(
									"[ProfileSync] Merging server profile into local:",
									onlineProfile.id,
									onlineProfile.name,
									"(server:",
									onlineProfile.updated_at,
									"local:",
									localProfile.hub_profile.updated,
									")",
								);

								localProfile.hub_profile.name = onlineProfile.name;
								localProfile.hub_profile.description =
									onlineProfile.description ?? null;
								localProfile.hub_profile.interests =
									onlineProfile.interests ?? [];
								localProfile.hub_profile.tags = onlineProfile.tags ?? [];
								localProfile.hub_profile.theme = onlineProfile.theme ?? null;
								localProfile.hub_profile.bits = onlineProfile.bit_ids ?? [];
								localProfile.hub_profile.apps = onlineProfile.apps ?? [];
								localProfile.hub_profile.hub = onlineProfile.hub;
								localProfile.hub_profile.hubs = onlineProfile.hubs ?? [];
								localProfile.hub_profile.settings =
									onlineProfile.settings ?? localProfile.hub_profile.settings;
								localProfile.hub_profile.updated = onlineProfile.updated_at;

								if (
									shouldReplaceWithServerImage(localProfile.hub_profile.icon) &&
									onlineProfile.icon
								) {
									localProfile.hub_profile.icon = onlineProfile.icon;
								}
								if (
									shouldReplaceWithServerImage(
										localProfile.hub_profile.thumbnail,
									) &&
									onlineProfile.thumbnail
								) {
									localProfile.hub_profile.thumbnail = onlineProfile.thumbnail;
								}

								localProfile.updated = onlineProfile.updated_at;

								try {
									await invoke("upsert_profile", {
										profile: localProfile,
									});
								} catch (error) {
									console.error(
										"[ProfileSync] Failed to merge profile:",
										onlineProfile.id,
										error,
									);
								}
							} else {
								// Local is newer or same — only update icon URLs if needed
								let needsUpdate = false;
								if (
									onlineProfile.icon &&
									shouldReplaceWithServerImage(localProfile.hub_profile.icon)
								) {
									localProfile.hub_profile.icon = onlineProfile.icon;
									needsUpdate = true;
								}
								if (
									onlineProfile.thumbnail &&
									shouldReplaceWithServerImage(
										localProfile.hub_profile.thumbnail,
									)
								) {
									localProfile.hub_profile.thumbnail = onlineProfile.thumbnail;
									needsUpdate = true;
								}
								if (needsUpdate) {
									await invoke("upsert_profile", {
										profile: localProfile,
									});
								}
							}
						}

						// Sync shortcuts
						if (onlineProfile.shortcuts) {
							const localShortcuts = await appsDB.shortcuts
								.where("profileId")
								.equals(onlineProfile.id)
								.toArray();
							const onlineShortcutMap = new Map(
								onlineProfile.shortcuts.map((s) => [s.id, s]),
							);
							for (const onlineShortcut of onlineProfile.shortcuts) {
								await appsDB.shortcuts.put(onlineShortcut);
							}
							for (const localShortcut of localShortcuts) {
								if (!onlineShortcutMap.has(localShortcut.id)) {
									await appsDB.shortcuts.delete(localShortcut.id);
								}
							}
						}
					}

					// If the current profile was deleted and we created new profiles from server, switch to the first one
					if (deletedCurrentProfile) {
						const finalProfiles =
							await invoke<Record<string, { hub_profile: IProfile }>>(
								"get_profiles_raw",
							);
						if (finalProfiles && Object.keys(finalProfiles).length > 0) {
							const firstProfileId = Object.keys(finalProfiles)[0];
							try {
								await invoke("set_current_profile", {
									profileId: firstProfileId,
								});
							} catch (error) {
								console.error(
									"[ProfileSync] Failed to set current profile after merge:",
									error,
								);
							}
						}
					}

					console.log("[ProfileSync] Profile sync complete");
				} catch (error) {
					console.warn("Failed to pull remote profiles:", error);
				}
			} catch (error) {
				console.error("[ProfileSync] Failed to sync profiles:", error);
			} finally {
				console.log("[ProfileSync] Sync finished, releasing lock");
				syncingRef.current = false;
			}
		};

		syncProfiles();

		const interval = setInterval(() => {
			if (syncingRef.current) return;
			syncProfiles();
		}, 5 * 60_000);
		return () => clearInterval(interval);
	}, [backend, isAuthenticated, accessToken, hubUrl]);

	return null;
}
