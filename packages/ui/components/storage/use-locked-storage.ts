"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
	useAppPermissions,
	useBackend,
	useBackendReady,
	useInvoke,
	useQueryClient,
} from "../..";
import { apiErrorMessage, isUpgradeRequiredError } from "../../lib/api-error";
import { isAbortError } from "../../lib/bulk-upload";
import { RolePermissions } from "../../lib/permission/role-permission";
import { asArray } from "../../lib/response-shape";
import type { IStorageItem } from "../../lib/schema/storage/storage-item";
import { encodeStorageSegment } from "../../lib/storage-tree";
import {
	type IListLockedItemsResponse,
	type ILockedItem,
	type ILockedItemDetail,
	type ILockedItemVersion,
	type ILockedStorageState,
	type ILockedStorageStatus,
	type ILockedUploadOptions,
	type ILockedUploadProgress,
	LOCKED_STORAGE_DISABLED,
	type LockedScope,
} from "../../state/backend-state/locked-storage-state";
import {
	lockedBrowserEntries,
	lockedFolderForPrefix,
	lockedScopeFor,
} from "./locked-storage-entries";
import { saveUrlAsFile } from "./save-download";
import {
	IDLE_UPLOAD_PROGRESS,
	type IUploadProgressState,
} from "./storage-upload-progress";

const LIST_PAGE_LIMIT = 500;
const LIST_MAX_PAGES = 20;
const STATUS_STALE_MS = 5 * 60_000;
/** A hub without the routes, or with the feature switched off, answers with these. */
const FEATURE_ABSENT_STATUSES = new Set([404, 405, 501, 503]);
const PHASE_SHARE: Record<ILockedUploadProgress["phase"], [number, number]> = {
	hashing: [0, 0.3],
	uploading: [0.3, 0.95],
	committing: [0.95, 1],
};

/** Query functions as methods, so `useInvoke` keys them by a name minifiers keep. */
class LockedStorageQueries {
	constructor(private readonly state: ILockedStorageState) {}

	async lockedStorageStatus(appId: string): Promise<ILockedStorageStatus> {
		try {
			return await this.state.getLockedStorageStatus(appId);
		} catch (error) {
			const status = (error as { status?: unknown } | null)?.status;
			if (typeof status === "number" && FEATURE_ABSENT_STATUSES.has(status)) {
				return { ...LOCKED_STORAGE_DISABLED };
			}
			throw error;
		}
	}

	async listLockedFolder(
		appId: string,
		scope: LockedScope,
		folder: string,
	): Promise<IListLockedItemsResponse> {
		const items: ILockedItem[] = [];
		const folders = new Set<string>();
		let cursor: string | null = null;
		for (let page = 0; page < LIST_MAX_PAGES; page++) {
			const response = await this.state.listLockedItems(appId, {
				scope,
				folder,
				limit: LIST_PAGE_LIMIT,
				cursor,
			});
			items.push(...(response.items ?? []));
			for (const name of response.folders ?? []) folders.add(name);
			cursor = response.next_cursor;
			if (!cursor) break;
		}
		return { items, folders: [...folders], next_cursor: cursor };
	}

	lockedItemDetail(appId: string, itemId: string): Promise<ILockedItemDetail> {
		return this.state.getLockedItem(appId, { item_id: itemId });
	}
}

export interface ILockedUploadTarget {
	readonly item?: ILockedItem;
}

export interface ILockedUploadSettings {
	readonly retentionDays?: number;
	readonly deleteAfterRetention?: boolean;
	readonly comment?: string;
}

interface IUploadJob {
	readonly file: File;
	readonly options: ILockedUploadOptions;
}

export interface ILockedConflict {
	readonly names: string[];
	readonly files: File[];
	readonly settings: ILockedUploadSettings;
}

export interface ILockedVersionTarget {
	readonly item: ILockedItem;
	readonly version: ILockedItemVersion;
}

export interface ILockedPurgeTarget {
	readonly item: ILockedItem;
	readonly versions: ILockedItemVersion[];
}

type WriteStorageItems = (
	items: { prefix: string; url?: string }[],
) => Promise<void>;

function overallProgress(
	completed: number,
	total: number,
	progress: ILockedUploadProgress,
): number {
	const [start, end] = PHASE_SHARE[progress.phase];
	const fraction = progress.total > 0 ? progress.loaded / progress.total : 1;
	const file = start + (end - start) * Math.min(1, fraction);
	return ((completed + file) / Math.max(1, total)) * 100;
}

export function useLockedStorage({
	appId,
	prefix,
	storageScopeKey,
	canWrite,
	storageItems,
	writeStorageItems,
}: Readonly<{
	appId: string;
	prefix: string;
	storageScopeKey: string;
	canWrite: boolean;
	storageItems: readonly IStorageItem[] | undefined;
	writeStorageItems?: WriteStorageItems;
}>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const backendReady = useBackendReady();
	const queryClient = useQueryClient();
	const permissions = useAppPermissions(appId);
	const state = backend.lockedStorageState;
	const queries = useMemo(() => new LockedStorageQueries(state), [state]);
	const scope = lockedScopeFor(storageScopeKey);
	const folder = lockedFolderForPrefix(prefix);

	const statusQuery = useInvoke(
		queries.lockedStorageStatus,
		queries,
		[appId],
		backendReady && !!appId,
		[],
		STATUS_STALE_MS,
	);
	const status = statusQuery.data?.enabled ? statusQuery.data : undefined;
	const enabled = !!status;
	const listing = useInvoke(
		queries.listLockedFolder,
		queries,
		[appId, scope, folder],
		enabled,
	);
	const [detailsItemId, setDetailsItemId] = useState<string | null>(null);
	const details = useInvoke(
		queries.lockedItemDetail,
		queries,
		[appId, detailsItemId ?? ""],
		enabled && !!detailsItemId,
	);

	const isAdmin = permissions.canStrict(RolePermissions.Admin);
	const canPurge = scope === "user" ? canWrite : isAdmin;
	const canExtendRetention =
		isAdmin &&
		!!status?.supports_retention_extension &&
		status.enforcement === "object";
	const canSetLegalHold = isAdmin && !!status?.supports_legal_hold;

	const listingData = enabled ? listing.data : undefined;
	const entries = useMemo(
		() => lockedBrowserEntries(prefix, listingData, asArray(storageItems)),
		[listingData, prefix, storageItems],
	);

	const refresh = useCallback(async () => {
		const keys = new Set([
			queries.listLockedFolder.name,
			queries.lockedItemDetail.name,
		]);
		await queryClient.invalidateQueries({
			predicate: ({ queryKey }) =>
				keys.has(queryKey[0] as string) && queryKey[1] === appId,
		});
	}, [appId, queries, queryClient]);

	const failure = useCallback((fallback: string, error: unknown) => {
		console.error(fallback, error);
		toast.error(fallback, {
			description: apiErrorMessage(
				error,
				error instanceof Error ? error.message : "",
			),
		});
	}, []);

	const downloadUrl = useCallback(
		(item: ILockedItem, version?: number) =>
			state.downloadLockedItem(appId, item.id, version),
		[appId, state],
	);

	const download = useCallback(
		async (item: ILockedItem, version?: number) => {
			try {
				const ticket = await downloadUrl(item, version);
				if (writeStorageItems) {
					await writeStorageItems([
						{ prefix: encodeStorageSegment(ticket.file_name), url: ticket.url },
					]);
					return;
				}
				await saveUrlAsFile(ticket.url, ticket.file_name);
			} catch (error) {
				failure(t("lockedStorage.downloadFailed", "Download failed"), error);
			}
		},
		[downloadUrl, failure, t, writeStorageItems],
	);

	const [uploadTarget, setUploadTarget] = useState<ILockedUploadTarget | null>(
		null,
	);
	const [conflict, setConflict] = useState<ILockedConflict | null>(null);
	const [progress, setProgress] =
		useState<IUploadProgressState>(IDLE_UPLOAD_PROGRESS);
	const [phase, setPhase] = useState<ILockedUploadProgress["phase"]>("hashing");
	const uploadAbort = useRef<AbortController | null>(null);

	const runUploads = useCallback(
		async (jobs: IUploadJob[]) => {
			if (jobs.length === 0) return;
			const controller = new AbortController();
			uploadAbort.current = controller;
			setProgress({
				isUploading: true,
				progress: 0,
				fileCount: jobs.length,
				currentFile: jobs[0]?.file.name ?? "",
			});
			let completed = 0;
			let lastError: unknown;
			const failed: string[] = [];
			try {
				for (const job of jobs) {
					if (controller.signal.aborted) break;
					try {
						await state.uploadLockedFile(appId, job.file, {
							...job.options,
							signal: controller.signal,
							onProgress: (update) => {
								setPhase(update.phase);
								setProgress((previous) => ({
									...previous,
									currentFile: job.file.name,
									progress: overallProgress(completed, jobs.length, update),
								}));
							},
						});
						completed++;
					} catch (error) {
						if (controller.signal.aborted || isAbortError(error)) break;
						failed.push(job.file.name);
						lastError = error;
						// A full quota refuses every later file the same way.
						if (isUpgradeRequiredError(error)) break;
					}
				}
			} finally {
				uploadAbort.current = null;
				setProgress(IDLE_UPLOAD_PROGRESS);
				await refresh();
			}
			if (controller.signal.aborted) {
				toast.info(t("lockedStorage.uploadCancelled", "Upload cancelled"));
			} else if (failed.length > 0) {
				failure(
					t("lockedStorage.uploadFailedFiles", "Could not lock {{files}}", {
						files: failed.join(", "),
					}),
					lastError,
				);
			} else {
				toast.success(
					t("lockedStorage.uploadDone", "Locked files stored: {{files}}", {
						files: completed,
					}),
				);
			}
		},
		[appId, failure, refresh, state, t],
	);

	const cancelUpload = useCallback(() => uploadAbort.current?.abort(), []);

	const jobsFor = useCallback(
		(
			files: File[],
			settings: ILockedUploadSettings,
			target: ILockedUploadTarget | null,
			overwrite: ReadonlySet<string>,
		): IUploadJob[] =>
			files.map((file) => ({
				file,
				options: {
					...(target?.item
						? { itemId: target.item.id }
						: {
								scope,
								folder,
								ifExists: overwrite.has(file.name.toLowerCase())
									? "new_version"
									: "error",
							}),
					retentionDays: settings.retentionDays,
					deleteAfterRetention: settings.deleteAfterRetention,
					comment: settings.comment,
				},
			})),
		[folder, scope],
	);

	const submitUpload = useCallback(
		(files: File[], settings: ILockedUploadSettings) => {
			const target = uploadTarget;
			setUploadTarget(null);
			if (!status || files.length === 0) return;
			const oversized = files.filter(
				(file) =>
					status.max_file_bytes > 0 && file.size > status.max_file_bytes,
			);
			if (oversized.length > 0) {
				toast.error(
					t("lockedStorage.fileTooLarge", "Too large to lock: {{files}}", {
						files: oversized.map((file) => file.name).join(", "),
					}),
				);
				return;
			}
			if (!target?.item) {
				const existing = new Set(
					(listingData?.items ?? []).map((item) => item.name.toLowerCase()),
				);
				const names = files
					.map((file) => file.name)
					.filter((name) => existing.has(name.toLowerCase()));
				if (names.length > 0) {
					setConflict({ names, files, settings });
					return;
				}
			}
			void runUploads(jobsFor(files, settings, target, new Set()));
		},
		[jobsFor, listingData, runUploads, status, t, uploadTarget],
	);

	const resolveConflict = useCallback(
		(uploadAsNewVersion: boolean) => {
			const pending = conflict;
			setConflict(null);
			if (!pending || !uploadAsNewVersion) return;
			const overwrite = new Set(
				pending.names.map((name) => name.toLowerCase()),
			);
			void runUploads(
				jobsFor(pending.files, pending.settings, null, overwrite),
			);
		},
		[conflict, jobsFor, runUploads],
	);

	const [retentionTarget, setRetentionTarget] =
		useState<ILockedVersionTarget | null>(null);
	const [purgeTarget, setPurgeTarget] = useState<ILockedPurgeTarget | null>(
		null,
	);
	const [mutating, setMutating] = useState(false);

	const mutate = useCallback(
		async (
			task: () => Promise<unknown>,
			success: string,
			failureMessage: string,
		) => {
			setMutating(true);
			try {
				await task();
				toast.success(success);
				return true;
			} catch (error) {
				failure(failureMessage, error);
				return false;
			} finally {
				setMutating(false);
				await refresh();
			}
		},
		[failure, refresh],
	);

	const extendRetention = useCallback(
		async (retainUntil: string) => {
			const target = retentionTarget;
			if (!target) return;
			const done = await mutate(
				() =>
					state.setLockedRetention(
						appId,
						target.item.id,
						target.version.id,
						retainUntil,
					),
				t("lockedStorage.retentionExtended", "Retention extended"),
				t("lockedStorage.retentionFailed", "Could not extend retention"),
			);
			if (done) setRetentionTarget(null);
		},
		[appId, mutate, retentionTarget, state, t],
	);

	const toggleLegalHold = useCallback(
		(target: ILockedVersionTarget) =>
			mutate(
				() =>
					state.setLockedLegalHold(
						appId,
						target.item.id,
						target.version.id,
						!target.version.legal_hold,
					),
				target.version.legal_hold
					? t("lockedStorage.legalHoldReleased", "Legal hold released")
					: t("lockedStorage.legalHoldPlaced", "Legal hold placed"),
				t("lockedStorage.legalHoldFailed", "Could not change the legal hold"),
			),
		[appId, mutate, state, t],
	);

	const toggleAutoDelete = useCallback(
		(target: ILockedVersionTarget) =>
			mutate(
				() =>
					state.setLockedAutoDelete(
						appId,
						target.item.id,
						target.version.id,
						!target.version.delete_after_retention,
					),
				target.version.delete_after_retention
					? t("lockedStorage.autoDeleteOff", "Kept after retention ends")
					: t(
							"lockedStorage.autoDeleteOn",
							"Deleted automatically when retention ends",
						),
				t(
					"lockedStorage.autoDeleteFailed",
					"Could not change automatic deletion",
				),
			),
		[appId, mutate, state, t],
	);

	const confirmPurge = useCallback(async () => {
		const target = purgeTarget;
		if (!target) return;
		const done = await mutate(
			async () => {
				for (const version of target.versions) {
					await state.purgeLockedItem(appId, target.item.id, version.id);
				}
			},
			t("lockedStorage.purged", "Stored bytes deleted, the record is kept"),
			t("lockedStorage.purgeFailed", "Could not delete the expired versions"),
		);
		if (done) setPurgeTarget(null);
	}, [appId, mutate, purgeTarget, state, t]);

	const closeDetails = useCallback(() => setDetailsItemId(null), []);
	const openUpload = useCallback(
		(target: ILockedUploadTarget = {}) => setUploadTarget(target),
		[],
	);
	const closeUpload = useCallback(() => setUploadTarget(null), []);
	const closeRetention = useCallback(() => setRetentionTarget(null), []);
	const closePurge = useCallback(() => setPurgeTarget(null), []);

	const progressLabel = {
		hashing: t("lockedStorage.phaseHashing", "Computing checksum"),
		uploading: t("lockedStorage.phaseUploading", "Uploading locked file"),
		committing: t("lockedStorage.phaseCommitting", "Verifying and locking"),
	}[phase];

	return {
		appId,
		queries,
		enabled,
		status,
		scope,
		folder,
		entries,
		canWrite,
		isAdmin,
		canPurge,
		canExtendRetention,
		canSetLegalHold,
		refresh,
		download,
		downloadUrl,
		detailsItemId,
		details,
		openDetails: setDetailsItemId,
		closeDetails,
		uploadTarget,
		openUpload,
		closeUpload,
		submitUpload,
		conflict,
		resolveConflict,
		progress,
		progressLabel,
		cancelUpload,
		retentionTarget,
		openRetention: setRetentionTarget,
		closeRetention,
		extendRetention,
		toggleLegalHold,
		toggleAutoDelete,
		purgeTarget,
		openPurge: setPurgeTarget,
		closePurge,
		confirmPurge,
		mutating,
	};
}

export type LockedStorageController = ReturnType<typeof useLockedStorage>;
