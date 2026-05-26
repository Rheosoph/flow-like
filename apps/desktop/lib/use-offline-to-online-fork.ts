"use client";

import { invoke } from "@tauri-apps/api/core";
import {
	IAppStatus,
	IAppVisibility,
	IVersionType,
	type IApp,
	type IBoard,
	type IEvent,
	type IMetadata,
	type IPage,
	type IWidget,
	type PageListItem,
	useBackend,
	useInvalidateInvoke,
} from "@flow-like/flow-like-ui";
import type {
	IBeginOnlineForkBody,
	IBeginOnlineForkResponse,
	IFinalizeOnlineForkResponse,
	IForkBundleSummary,
} from "@flow-like/flow-like-ui/lib/schema/app/fork";
import type { IProfileApp } from "@flow-like/flow-like-ui/lib/schema/profile/profile";
import { useRouter } from "next/navigation";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { appsDB } from "./apps-db";

interface UploadLocalAppContentResponse {
	content_objects_copied: number;
	content_bytes_copied: number;
	failures: { kind_and_path: string; reason: string }[];
}

const LANGUAGE = "en";
const TOKEN_FIELD_NAMES = new Set([
	"access_token",
	"auth_token",
	"id_token",
	"oauth_tokens",
	"pat",
	"personal_access_token",
	"refresh_token",
]);

function createProfileApp(appId: string): IProfileApp {
	return {
		app_id: appId,
		favorite: false,
		favorite_order: null,
		pinned: false,
		pinned_order: null,
	};
}

function cloneValue<T>(value: T): T {
	if (typeof structuredClone === "function") {
		return structuredClone(value);
	}
	return JSON.parse(JSON.stringify(value)) as T;
}

function systemTimeNow() {
	const ms = Date.now();
	return {
		secs_since_epoch: Math.floor(ms / 1000),
		nanos_since_epoch: (ms % 1000) * 1_000_000,
	};
}

function stripForkSecrets<T>(value: T): T {
	const clone = cloneValue(value);
	stripForkSecretsInPlace(clone, new WeakSet<object>());
	return clone;
}

function stripForkSecretsInPlace(value: unknown, seen: WeakSet<object>) {
	if (!value || typeof value !== "object") return;
	if (seen.has(value)) return;
	seen.add(value);

	const record = value as Record<string, unknown>;
	if (record.secret === true) {
		if ("default_value" in record) record.default_value = null;
		if ("defaultValue" in record) record.defaultValue = null;
	}

	for (const key of Object.keys(record)) {
		const lowerKey = key.toLowerCase();
		if (TOKEN_FIELD_NAMES.has(lowerKey) || lowerKey.startsWith("secret_")) {
			record[key] = null;
			continue;
		}
		stripForkSecretsInPlace(record[key], seen);
	}
}

function prepareAppForOnlineFork(sourceApp: IApp, newAppId: string): IApp {
	const app = stripForkSecrets(sourceApp);
	app.id = newAppId;
	app.visibility = IAppVisibility.Private;
	app.status = IAppStatus.Active;
	app.allow_forking = false;
	app.forked_from = sourceApp.id;
	app.forked_at = systemTimeNow();
	app.updated_at = systemTimeNow();
	return app;
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

function uniquePages(pages: PageListItem[]): PageListItem[] {
	const seen = new Set<string>();
	const result: PageListItem[] = [];
	for (const page of pages) {
		const key = `${page.boardId ?? ""}:${page.pageId}`;
		if (seen.has(key)) continue;
		seen.add(key);
		result.push(page);
	}
	return result;
}

export function useOfflineToOnlineFork() {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const router = useRouter();
	const [isForking, setIsForking] = useState(false);

	const forkOfflineAppOnline = useCallback(
		async (sourceAppId: string, appName: string) => {
			if (isForking) return;
			setIsForking(true);
			const toastId = toast.loading(`Creating online copy of ${appName}...`);

			try {
				const settingsProfile = await backend.userState.getSettingsProfile();
				const profile = settingsProfile.hub_profile;
				const summary = await invoke<IForkBundleSummary>(
					"summarize_local_app_bundle",
					{ appId: sourceAppId },
				);

				const beginBody: IBeginOnlineForkBody = {
					source_app_id: sourceAppId,
					summary,
					language: LANGUAGE,
				};
				const begin = await backend.apiState.post<IBeginOnlineForkResponse>(
					profile,
					"apps/fork/online/begin",
					beginBody,
				);

				const sourceApp = await backend.appState.getApp(sourceAppId);
				const appMeta = await backend.appState.getAppMeta(
					sourceAppId,
					LANGUAGE,
				);
				const boards = await backend.boardState.getBoards(sourceAppId);
				const events = await backend.eventState.getEvents(sourceAppId);
				const widgetEntries = await backend.widgetState.getWidgets(
					sourceAppId,
					LANGUAGE,
				);
				const templateEntries = await backend.templateState.getTemplates(
					sourceAppId,
					LANGUAGE,
				);
				const pageList = uniquePages(await backend.pageState.getPages(sourceAppId));

				const contentUpload = await invoke<UploadLocalAppContentResponse>(
					"upload_local_app_content_bundle",
					{
						args: {
							source_app_id: sourceAppId,
							destination_content_prefix: begin.upload_path,
							credentials: begin.shared_credentials,
						},
					},
				);

				if (contentUpload.failures.length > 0) {
					throw new Error(
						`${contentUpload.failures.length} content file(s) failed to upload`,
					);
				}

				const preparedWidgets: Array<{
					widget: IWidget;
					metadata?: IMetadata;
				}> = [];
				for (const [, widgetId, listedMeta] of widgetEntries) {
					const widget = await backend.widgetState.getWidget(
						sourceAppId,
						widgetId,
					);
					let metadata = listedMeta;
					if (!metadata) {
						metadata = await backend.widgetState
							.getWidgetMeta(sourceAppId, widgetId, LANGUAGE)
							.catch(() => undefined);
					}
					preparedWidgets.push({
						widget: stripForkSecrets(widget),
						metadata: metadata ? stripForkSecrets(metadata) : undefined,
					});
				}

				const preparedPages: IPage[] = [];
				for (const page of pageList) {
					preparedPages.push(
						stripForkSecrets(
							await backend.pageState.getPage(
								sourceAppId,
								page.pageId,
								page.boardId,
							),
						),
					);
				}

				const preparedTemplates: Array<{
					templateId: string;
					board: IBoard;
					metadata?: IMetadata;
				}> = [];
				for (const [, templateId, listedMeta] of templateEntries) {
					const board = await backend.templateState.getTemplate(
						sourceAppId,
						templateId,
					);
					let metadata = listedMeta;
					if (!metadata) {
						metadata = await backend.templateState
							.getTemplateMeta(sourceAppId, templateId, LANGUAGE)
							.catch(() => undefined);
					}
					preparedTemplates.push({
						templateId,
						board: stripForkSecrets(board),
						metadata: metadata ? stripForkSecrets(metadata) : undefined,
					});
				}

				for (const board of boards) {
					const preparedBoard = stripForkSecrets(board);
					await backend.apiState.put<{ id: string }>(
						profile,
						`apps/${begin.new_app_id}/board/${preparedBoard.id}`,
						{
							name: preparedBoard.name,
							description: preparedBoard.description,
							log_level: preparedBoard.log_level,
							stage: preparedBoard.stage,
							execution_mode: preparedBoard.execution_mode,
							template: preparedBoard,
						},
					);
				}

				for (const { widget, metadata } of preparedWidgets) {
					await backend.apiState.put<IWidget>(
						profile,
						`apps/${begin.new_app_id}/widgets/${widget.id}`,
						{ widget },
					);
					if (metadata) {
						await backend.apiState.put<void>(
							profile,
							`apps/${begin.new_app_id}/meta?language=${LANGUAGE}&widget_id=${widget.id}`,
							metadata,
						);
					}
				}

				for (const page of preparedPages) {
					await backend.apiState.put<IPage>(
						profile,
						`apps/${begin.new_app_id}/pages/${page.id}`,
						{ page },
					);
				}

				for (const event of events) {
					const preparedEvent = stripForkSecrets(event) as IEvent;
					await backend.apiState.put<IEvent>(
						profile,
						`apps/${begin.new_app_id}/events/${preparedEvent.id}`,
						{
							event: preparedEvent,
							profile_id: profile.id,
						},
					);
				}

				const destinationBoardIds = new Set(boards.map((board) => board.id));
				for (const { templateId, board, metadata } of preparedTemplates) {
					if (!destinationBoardIds.has(board.id)) {
						console.warn(
							"Skipping template during offline-to-online fork because its board is not present:",
							{ templateId, boardId: board.id },
						);
						continue;
					}
					await backend.apiState.put<[string, [number, number, number]]>(
						profile,
						`apps/${begin.new_app_id}/templates/${templateId}`,
						{
							board_id: board.id,
							board_version: board.version,
							version_type: IVersionType.Patch,
						},
					);
					if (metadata) {
						await backend.apiState.put<void>(
							profile,
							`apps/${begin.new_app_id}/meta?language=${LANGUAGE}&template_id=${templateId}`,
							metadata,
						);
					}
				}

				const preparedApp = prepareAppForOnlineFork(sourceApp, begin.new_app_id);
				await backend.apiState.put<IApp>(profile, `apps/${begin.new_app_id}`, {
					app: preparedApp,
				});
				await backend.apiState.put<void>(
					profile,
					`apps/${begin.new_app_id}/meta?language=${LANGUAGE}`,
					stripForkSecrets(appMeta),
				);

				const finalized =
					await backend.apiState.post<IFinalizeOnlineForkResponse>(
						profile,
						`apps/${begin.new_app_id}/fork/online/finalize`,
						{ visibility: "private" },
					);

				await backend.userState.updateProfileApp(
					settingsProfile,
					createProfileApp(begin.new_app_id),
					"Upsert",
				);
				await appsDB.visibility.put({
					appId: begin.new_app_id,
					visibility: IAppVisibility.Private,
				});

				await Promise.all([
					invalidate(backend.userState.getProfile, []),
					invalidate(backend.userState.getSettingsProfile, []),
					invalidate(backend.userState.getProfiles, []),
					invalidate(backend.appState.getApps, []),
					invalidate(backend.appState.getApp, [begin.new_app_id]),
					invalidate(backend.appState.getAppMeta, [begin.new_app_id]),
				]);

				toast.success(
					`Online fork created (${contentUpload.content_objects_copied} content files, ${formatBytes(
						finalized.total_size_bytes,
					)})`,
					{ id: toastId },
				);
				router.push(`/library/config?id=${begin.new_app_id}`);
			} catch (error) {
				const message =
					error instanceof Error
						? error.message
						: String(error ?? "Failed to create online fork");
				toast.error(`Couldn't create online fork: ${message}`, { id: toastId });
			} finally {
				setIsForking(false);
			}
		},
		[backend, invalidate, isForking, router],
	);

	return { forkOfflineAppOnline, isForking };
}
