"use client";

import {
	IAppVisibility,
	useBackend,
	useInvalidateInvoke,
} from "@flow-like/flow-like-ui";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { useRouter } from "next/navigation";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { appsDB } from "./apps-db";
import { runtimeVarsDB } from "./runtime-vars-db";

export interface IDuplicateReport {
	app_id: string;
	source_app_id: string;
	/** Source board id → the copy's board id. */
	board_ids: Record<string, string>;
	pages: number;
	events: number;
	widgets: number;
	templates: number;
	files_copied: number;
	bytes_copied: number;
	warnings: string[];
}

const LISTED_WARNINGS = 3;

/** Runtime variable values live only in this device's IndexedDB, keyed by app
 * and board, so the backend copy cannot carry them. Variable ids survive the
 * duplicate unchanged; only the app and board ids move. */
async function copyRuntimeVariables(report: IDuplicateReport): Promise<void> {
	const values = await runtimeVarsDB.values
		.where("appId")
		.equals(report.source_app_id)
		.toArray();
	const updatedAt = new Date().toISOString();
	const copies = values.flatMap((value) => {
		const boardId = report.board_ids[value.boardId];
		if (!boardId) return [];
		return [
			{
				...value,
				id: `${report.app_id}:${value.variableId}`,
				appId: report.app_id,
				boardId,
				updatedAt,
			},
		];
	});
	if (copies.length > 0) await runtimeVarsDB.values.bulkPut(copies);
}

/**
 * Duplicates an offline app into a new offline app on this device. The
 * backend copies and re-keys everything on disk; this hook carries over the
 * device-local state keyed by app, refreshes the library and reports the
 * outcome.
 */
export function useDuplicateLocalApp() {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const router = useRouter();
	const { t } = useTranslation("common");
	const [isDuplicating, setIsDuplicating] = useState(false);

	const refreshLibrary = useCallback(
		(appId: string) =>
			Promise.all([
				invalidate(backend.userState.getProfile, []),
				invalidate(backend.userState.getSettingsProfile, []),
				invalidate(backend.userState.getProfiles, []),
				invalidate(backend.appState.getApps, []),
				invalidate(backend.appState.getApp, [appId]),
				invalidate(backend.appState.getAppMeta, [appId]),
			]),
		[backend, invalidate],
	);

	const duplicate = useCallback(
		async (sourceAppId: string, name: string) => {
			if (isDuplicating) return;
			setIsDuplicating(true);
			try {
				const report = await invoke<IDuplicateReport>("duplicate_local_app", {
					appId: sourceAppId,
					name,
				});
				const warnings = [...report.warnings];
				await appsDB.visibility.put({
					appId: report.app_id,
					visibility: IAppVisibility.Offline,
				});
				try {
					await copyRuntimeVariables(report);
				} catch (error) {
					console.warn("Copying runtime variable values failed", error);
					warnings.unshift(
						t(
							"runtimeVariableValuesWereNotCopiedSetThemAgainInTheCopy",
							"Runtime variable values were not copied. Set them again in the copy.",
						),
					);
				}
				await refreshLibrary(report.app_id);

				const title = t("duplicatedApp", "Duplicated {{name}}", { name });
				const action = {
					label: t("open", "Open"),
					onClick: () =>
						router.push(
							`/library/config?id=${encodeURIComponent(report.app_id)}`,
						),
				};
				if (warnings.length === 0) {
					toast.success(title, { action });
				} else {
					toast.warning(title, {
						action,
						duration: 10_000,
						description: [
							t("someItemsCouldNotBeCopied", "Some items could not be copied:"),
							...warnings.slice(0, LISTED_WARNINGS),
						].join(" · "),
					});
				}
			} catch (error) {
				toast.error(
					t("couldNotDuplicateTheApp", "Could not duplicate the app"),
					{ description: getErrorMessage(error) },
				);
			} finally {
				setIsDuplicating(false);
			}
		},
		[isDuplicating, refreshLibrary, router, t],
	);

	return { duplicate, isDuplicating };
}
