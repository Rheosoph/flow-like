"use client";

import {
	IAppVisibility,
	useBackend,
	useInvalidateInvoke,
} from "@flow-like/flow-like-ui";
import type { IBeginForkResponse } from "@flow-like/flow-like-ui/components/settings/forking/fork-app-dialog";
import type {
	IBeginOfflineForkResponse,
	IOfflineForkFilesPage,
} from "@flow-like/flow-like-ui/lib/schema/app/fork";
import type { IProfileApp } from "@flow-like/flow-like-ui/lib/schema/profile/profile";
import { invoke } from "@tauri-apps/api/core";
import { useCallback } from "react";
import { toast } from "sonner";
import { appsDB } from "./apps-db";

interface FileFailure {
	kind_and_path: string;
	reason: string;
}

interface ApplyForkBundleResponse {
	app_id: string;
	meta_files_written: number;
	/** Tables recreated empty for a schema-only database fork. */
	db_tables_created: number;
	failures: FileFailure[];
}

interface ForkFilesResponse {
	files_copied: number;
	bytes_copied: number;
	failures: FileFailure[];
}

interface ContentTally {
	files: number;
	bytes: number;
	/** Files the owner's fork policy kept out. */
	skipped: number;
	failures: FileFailure[];
}

type BackendState = ReturnType<typeof useBackend>;

function isOfflineForkResponse(
	res: IBeginForkResponse,
): res is IBeginOfflineForkResponse {
	return "meta_files" in res;
}

function createProfileApp(appId: string): IProfileApp {
	return {
		app_id: appId,
		favorite: false,
		favorite_order: null,
		pinned: false,
		pinned_order: null,
	};
}

/** Trailing ", 5 skipped by fork policy, 2 tables created" — empty when
 * the owner's policy excluded nothing. */
function formatPolicyEffects(skipped: number, tablesCreated: number): string {
	const parts: string[] = [];
	if (skipped > 0) parts.push(`${skipped} skipped by fork policy`);
	if (tablesCreated > 0)
		parts.push(
			`${tablesCreated} empty table${tablesCreated === 1 ? "" : "s"} created`,
		);
	return parts.length > 0 ? `, ${parts.join(", ")}` : "";
}

function formatBytes(bytes: number): string {
	if (bytes === 0) return "0 B";
	const units = ["B", "KB", "MB", "GB", "TB"];
	const i = Math.min(
		units.length - 1,
		Math.floor(Math.log(bytes) / Math.log(1024)),
	);
	const value = bytes / 1024 ** i;
	return `${value.toFixed(value >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}

/**
 * Pulls the source's content files page by page. The hub signs one download
 * per file and has already applied the owner's fork policy and the paths
 * the files take in the fork; a staged meta artifact wins over a content
 * file with the same path.
 */
async function downloadForkContent(
	backend: BackendState,
	response: IBeginOfflineForkResponse,
): Promise<ContentTally> {
	const profile = (await backend.userState.getSettingsProfile()).hub_profile;
	const skipPaths = response.meta_files.map((file) => file.path);
	const tally: ContentTally = { files: 0, bytes: 0, skipped: 0, failures: [] };
	let after: string | undefined;
	do {
		const query = new URLSearchParams({ seed: response.new_app_id });
		if (after) query.set("after", after);
		const page = await backend.apiState.get<IOfflineForkFilesPage>(
			profile,
			`apps/${response.source_app_id}/fork/offline/files?${query}`,
		);
		tally.skipped += page.skipped;
		if (page.files.length > 0) {
			const copied = await invoke<ForkFilesResponse>("download_fork_files", {
				args: {
					app_id: response.new_app_id,
					files: page.files,
					skip_paths: skipPaths,
				},
			});
			tally.files += copied.files_copied;
			tally.bytes += copied.bytes_copied;
			tally.failures.push(...copied.failures);
		}
		after = page.next ?? undefined;
	} while (after);
	return tally;
}

async function applyOfflineFork(
	backend: BackendState,
	response: IBeginOfflineForkResponse,
): Promise<void> {
	const applied = await invoke<ApplyForkBundleResponse>("apply_fork_bundle", {
		args: {
			app_id: response.new_app_id,
			meta_files: response.meta_files,
			db_table_schemas: response.db_table_schemas ?? [],
		},
	});
	await appsDB.visibility.put({
		appId: response.new_app_id,
		visibility: IAppVisibility.Offline,
	});
	const content = await downloadForkContent(backend, response);

	const totalCopied = applied.meta_files_written + content.files;
	const failures = applied.failures.length + content.failures.length;
	const policyEffects = formatPolicyEffects(
		content.skipped,
		applied.db_tables_created,
	);
	if (failures === 0) {
		toast.success(
			`Applied ${totalCopied} files (${formatBytes(content.bytes)} content)${policyEffects}`,
		);
	} else {
		toast.warning(
			`Applied ${totalCopied} files${policyEffects}, ${failures} failed — re-open the dialog to retry`,
		);
	}

	if (response.content_store_leaks && response.content_store_leaks.length > 0) {
		toast.warning(
			`Source app has ${response.content_store_leaks.length} suspicious file(s) in the content store — operator should investigate`,
		);
	}
}

/**
 * Desktop fork follow-up. Offline forks download the hub's signed files
 * into the local stores; online forks already exist on the server, but still
 * need a local profile entry so the desktop library reflects the fork
 * immediately. Both paths update local visibility and invalidate the
 * profile/app caches.
 *
 * Failures are surfaced via toast — while the signed downloads are valid
 * the user can re-open the dialog to retry.
 */
export function useApplyForkBundle() {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();

	return useCallback(
		async (response: IBeginForkResponse) => {
			try {
				if (isOfflineForkResponse(response)) {
					await applyOfflineFork(backend, response);
				} else {
					const profile = await backend.userState.getSettingsProfile();
					await backend.userState.updateProfileApp(
						profile,
						createProfileApp(response.new_app_id),
						"Upsert",
					);
					await appsDB.visibility.put({
						appId: response.new_app_id,
						visibility: IAppVisibility.Private,
					});
				}

				await Promise.all([
					invalidate(backend.userState.getProfile, []),
					invalidate(backend.userState.getSettingsProfile, []),
					invalidate(backend.userState.getProfiles, []),
					invalidate(backend.appState.getApps, []),
					invalidate(backend.appState.getApp, [response.new_app_id]),
					invalidate(backend.appState.getAppMeta, [response.new_app_id]),
				]);
			} catch (err) {
				const message =
					err instanceof Error
						? err.message
						: String(err ?? "Failed to finish desktop fork handling");
				toast.error(`Fork desktop follow-up failed: ${message}`);
			}
		},
		[backend, invalidate],
	);
}
