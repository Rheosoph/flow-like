"use client";

import { invoke } from "@tauri-apps/api/core";
import { IAppVisibility } from "@tm9657/flow-like-ui";
import type { IBeginForkResponse } from "@tm9657/flow-like-ui/components/settings/forking/fork-app-dialog";
import type { IBeginOfflineForkResponse } from "@tm9657/flow-like-ui/lib/schema/app/fork";
import { useCallback } from "react";
import { toast } from "sonner";
import { appsDB } from "./apps-db";

interface ApplyForkBundleResponse {
	app_id: string;
	meta_blobs_written: number;
	content_objects_copied: number;
	content_bytes_copied: number;
	failures: { kind_and_path: string; reason: string }[];
}

function isOfflineForkResponse(
	res: IBeginForkResponse,
): res is IBeginOfflineForkResponse {
	return "meta_blobs" in res;
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
 * Desktop's offline fork follow-up: once the server materializes the
 * bundle, build an object_store client from the scoped read credentials
 * and pull the entire `bundle_prefix` in one go. The new app id is
 * recorded in the local visibility table so the fork shows up in the
 * apps list. Failures are surfaced via toast — within the credential's
 * expiration window the user can re-open the dialog to retry the same
 * bundle without paying for a fresh server-side materialization.
 */
export function useApplyForkBundle() {
	return useCallback(async (response: IBeginForkResponse) => {
		if (!isOfflineForkResponse(response)) return;
		try {
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
					},
				},
			);

			await appsDB.visibility.put({
				appId: response.new_app_id,
				visibility: IAppVisibility.Offline,
			});

			const totalCopied =
				result.meta_blobs_written + result.content_objects_copied;
			if (result.failures.length === 0) {
				toast.success(
					`Applied ${totalCopied} files (${formatBytes(
						result.content_bytes_copied,
					)} content)`,
				);
			} else {
				toast.warning(
					`Applied ${totalCopied} files, ${result.failures.length} failed — re-open the dialog to retry`,
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
		} catch (err) {
			const message =
				err instanceof Error
					? err.message
					: String(err ?? "Failed to apply fork bundle");
			toast.error(`Fork bundle apply failed: ${message}`);
		}
	}, []);
}
