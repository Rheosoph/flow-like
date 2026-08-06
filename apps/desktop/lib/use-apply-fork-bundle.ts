"use client";

import { invoke } from "@tauri-apps/api/core";
import {
	IAppVisibility,
	useBackend,
	useInvalidateInvoke,
} from "@flow-like/flow-like-ui";
import type { IBeginForkResponse } from "@flow-like/flow-like-ui/components/settings/forking/fork-app-dialog";
import type { IBeginOfflineForkResponse } from "@flow-like/flow-like-ui/lib/schema/app/fork";
import type { IProfileApp } from "@flow-like/flow-like-ui/lib/schema/profile/profile";
import { useCallback } from "react";
import { toast } from "sonner";
import { appsDB } from "./apps-db";

interface ApplyForkBundleResponse {
	app_id: string;
	meta_blobs_written: number;
	content_objects_copied: number;
	content_bytes_copied: number;
	/** Objects the owner's fork policy excluded from the mirror. */
	content_objects_skipped: number;
	/** Tables recreated empty for a schema-only database fork. */
	db_tables_created: number;
	failures: { kind_and_path: string; reason: string }[];
}

function isOfflineForkResponse(
	res: IBeginForkResponse,
): res is IBeginOfflineForkResponse {
	return "meta_blobs" in res;
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
function formatPolicyEffects(result: ApplyForkBundleResponse): string {
	const parts: string[] = [];
	if (result.content_objects_skipped > 0)
		parts.push(`${result.content_objects_skipped} skipped by fork policy`);
	if (result.db_tables_created > 0)
		parts.push(
			`${result.db_tables_created} empty table${
				result.db_tables_created === 1 ? "" : "s"
			} created`,
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
 * Desktop fork follow-up. Offline forks apply the signed bundle into
 * the local stores; online forks already exist on the server, but still
 * need a local profile entry so the desktop library reflects the fork
 * immediately. Both paths update local visibility and invalidate the
 * profile/app caches.
 *
 * Failures are surfaced via toast — within the credential's
 * expiration window the user can re-open the dialog to retry the same
 * bundle without paying for a fresh server-side materialization.
 */
export function useApplyForkBundle() {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();

	return useCallback(
		async (response: IBeginForkResponse) => {
			try {
				if (isOfflineForkResponse(response)) {
					const idMap = response.report.id_map as {
						widgets?: Record<string, string>;
						templates?: Record<string, string>;
						pages?: Record<string, string>;
					};
					const result = await invoke<ApplyForkBundleResponse>(
						"apply_fork_bundle",
						{
							args: {
								app_id: response.new_app_id,
								meta_blobs: response.meta_blobs,
								source_content_prefix: response.source_content_prefix,
								credentials: response.shared_credentials,
								widget_id_map: idMap?.widgets ?? {},
								template_id_map: idMap?.templates ?? {},
								page_id_map: idMap?.pages ?? {},
								content_exclude_prefixes:
									response.content_exclude_prefixes ?? [],
								db_table_schemas: response.db_table_schemas ?? [],
							},
						},
					);

					await appsDB.visibility.put({
						appId: response.new_app_id,
						visibility: IAppVisibility.Offline,
					});

					const totalCopied =
						result.meta_blobs_written + result.content_objects_copied;
					const policyEffects = formatPolicyEffects(result);
					if (result.failures.length === 0) {
						toast.success(
							`Applied ${totalCopied} files (${formatBytes(
								result.content_bytes_copied,
							)} content)${policyEffects}`,
						);
					} else {
						toast.warning(
							`Applied ${totalCopied} files${policyEffects}, ${result.failures.length} failed — re-open the dialog to retry`,
						);
					}

					if (
						response.content_store_leaks &&
						response.content_store_leaks.length > 0
					) {
						toast.warning(
							`Source app has ${response.content_store_leaks.length} suspicious file(s) in the content store — operator should investigate`,
						);
					}
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
