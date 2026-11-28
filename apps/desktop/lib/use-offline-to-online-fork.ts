"use client";

import {
	type IApp,
	IAppVisibility,
	type IBoard,
	type IEvent,
	type IEventRegistration,
	type IMetadata,
	type IPage,
	IVersionType,
	type IWidget,
	type PageListItem,
	normalizePageForPersistence,
	normalizeWidgetForPersistence,
	useBackend,
	useInvalidateInvoke,
} from "@flow-like/flow-like-ui";
import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import type {
	IAdoptUploadResponse,
	IBeginOnlineForkBody,
	IBeginOnlineForkResponse,
	IFinalizeOnlineForkAppSettings,
	IFinalizeOnlineForkResponse,
	IForkBundleSummary,
	IUploadIdInventory,
	IUploadUrlsResponse,
} from "@flow-like/flow-like-ui/lib/schema/app/fork";
import type {
	IProfile,
	IProfileApp,
} from "@flow-like/flow-like-ui/lib/schema/profile/profile";
import type { IApiState } from "@flow-like/flow-like-ui/state/backend-state/api-state";
import type { IBackendRole } from "@flow-like/flow-like-ui/state/backend-state/types";
import { invoke } from "@tauri-apps/api/core";
import { useRouter } from "next/navigation";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { appsDB } from "./apps-db";

interface ForkFilesResponse {
	files_copied: number;
	bytes_copied: number;
	failures: { kind_and_path: string; reason: string }[];
}

interface LocalContentFile {
	path: string;
	size: number;
}

/** Most files one signed-URL request may ask for (`MAX_TRANSFER_BATCH`). */
const UPLOAD_BATCH = 500;

const LANGUAGE = "en";

/**
 * Puts the local app's content files through signed upload URLs into the
 * hub's staging area, then has the hub move them into the new app, where
 * they are re-keyed with the rest of the upload. No storage credential
 * reaches the device.
 */
const uploadLocalContent = async (
	api: IApiState,
	profile: IProfile,
	sourceAppId: string,
	newAppId: string,
): Promise<ForkFilesResponse> => {
	const files = await invoke<LocalContentFile[]>("list_local_app_content", {
		appId: sourceAppId,
	});
	const total: ForkFilesResponse = {
		files_copied: 0,
		bytes_copied: 0,
		failures: [],
	};
	for (let start = 0; start < files.length; start += UPLOAD_BATCH) {
		const signed = await api.post<IUploadUrlsResponse>(
			profile,
			`apps/${newAppId}/fork/online/upload-urls`,
			{ files: files.slice(start, start + UPLOAD_BATCH) },
		);
		const uploaded = await invoke<ForkFilesResponse>("upload_local_app_files", {
			args: { source_app_id: sourceAppId, files: signed.files },
		});
		total.files_copied += uploaded.files_copied;
		total.bytes_copied += uploaded.bytes_copied;
		total.failures.push(...uploaded.failures);
	}
	let adopt: IAdoptUploadResponse;
	do {
		adopt = await api.post<IAdoptUploadResponse>(
			profile,
			`apps/${newAppId}/fork/online/adopt`,
		);
	} while (adopt.remaining);
	return total;
};
const TOKEN_FIELD_NAMES = new Set([
	"access_token",
	"auth_token",
	"id_token",
	"oauth_tokens",
	"pat",
	"personal_access_token",
	"refresh_token",
]);

/**
 * Roles the destination already owns — `/fork/online/begin` inserts a
 * fresh Owner / Admin / User trio. Anything the source added on top is
 * copied; the server rejects roles carrying the Owner bit outright, so
 * those surface as skips.
 */
const SYSTEM_ROLE_NAMES = new Set(["Owner", "Admin", "User"]);

/**
 * Local sink registrations for one app, keyed by event id. Sinks are
 * auto-created server-side from the event on upsert and run as the user
 * who saves it, so the registrations only tell which events had OAuth
 * grants that the fork has to reconnect.
 */
async function loadLocalSinks(
	sinkState: { listEventSinks(): Promise<IEventRegistration[]> } | undefined,
	appId: string,
): Promise<Map<string, IEventRegistration>> {
	if (!sinkState) return new Map();
	try {
		const registrations = await sinkState.listEventSinks();
		return new Map(
			registrations
				.filter((registration) => registration.app_id === appId)
				.map((registration) => [registration.event_id, registration]),
		);
	} catch (error) {
		console.warn("Could not read local event sinks during fork:", error);
		return new Map();
	}
}

interface LocalAppContents {
	readonly appId: string;
	readonly boards: readonly IBoard[];
	readonly events: readonly IEvent[];
	readonly pages: readonly PageListItem[];
	readonly widgetIds: readonly string[];
	readonly templateIds: readonly string[];
}

/**
 * Every id of the local app. The server mints the uploaded app's ids from
 * them and re-keys every board, event, page and widget pushed afterwards, so
 * it has to know all of them, including nodes and pins other boards point at.
 */
const collectUploadIds = (local: LocalAppContents): IUploadIdInventory => {
	const nodes = new Set<string>();
	const pins = new Set<string>();
	const layers = new Set<string>();
	const addPins = (graphPins: Readonly<Record<string, { id: string }>>) => {
		for (const pin of Object.values(graphPins)) pins.add(pin.id);
	};
	const addNodes = (graphNodes: IBoard["nodes"]) => {
		for (const node of Object.values(graphNodes)) {
			nodes.add(node.id);
			addPins(node.pins);
		}
	};
	for (const board of local.boards) {
		addNodes(board.nodes);
		for (const layer of Object.values(board.layers)) {
			layers.add(layer.id);
			addNodes(layer.nodes);
			addPins(layer.pins);
		}
	}
	return {
		app_id: local.appId,
		boards: local.boards.map((board) => board.id),
		nodes: [...nodes],
		pins: [...pins],
		layers: [...layers],
		events: local.events.map((event) => event.id),
		pages: local.pages.map((page) => page.pageId),
		widgets: [...local.widgetIds],
		templates: [...local.templateIds],
	};
};

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

function getFinalizeAppSettings(
	sourceApp: IApp,
): IFinalizeOnlineForkAppSettings {
	return {
		changelog: sourceApp.changelog ?? null,
		primary_category: sourceApp.primary_category ?? null,
		secondary_category: sourceApp.secondary_category ?? null,
		price: sourceApp.price ?? null,
		version: sourceApp.version ?? null,
		execution_mode: sourceApp.execution_mode,
	};
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

/** Why the hub refused to pin a package on the new online copy. */
function packageSkipReason(error: unknown): string {
	if (!(error instanceof ApiResponseError)) return "couldn't add it";
	if (error.code === "PACKAGE_LICENSE_REQUIRED") return "paid, not owned";
	if (error.code === "PACKAGE_ACCESS_REQUIRED") return "no access";
	return error.status === 404 ? "no longer in the registry" : "couldn't add it";
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
				const pageList = uniquePages(
					await backend.pageState.getPages(sourceAppId),
				);

				// The server mints every id of the copy from this list and
				// re-keys everything pushed below; the ids sent with each
				// request stay the local ones.
				const beginBody: IBeginOnlineForkBody = {
					source_app_id: sourceApp.forked_from ?? sourceAppId,
					id_inventory: collectUploadIds({
						appId: sourceAppId,
						boards,
						events,
						pages: pageList,
						widgetIds: widgetEntries.map(([, widgetId]) => widgetId),
						templateIds: templateEntries.map(([, templateId]) => templateId),
					}),
					fork_id_seeds: sourceApp.forked_from
						? (sourceApp.fork_id_seeds ?? null)
						: [],
					summary,
					language: LANGUAGE,
				};
				const begin = await backend.apiState.post<IBeginOnlineForkResponse>(
					profile,
					"apps/fork/online/begin",
					beginBody,
				);

				if (begin.project_limit_notice) toast.info(begin.project_limit_notice);

				// Best-effort extras. None of these are load-bearing for the
				// fork itself, so every lookup degrades to "nothing to carry"
				// rather than failing the whole operation: a purely local app
				// has no server-side roles, its packages may not resolve on
				// this deployment, and its sinks may hold credentials that
				// cannot travel.
				const localPackages = await invoke<Record<string, string>>(
					"app_list_packages",
					{ appId: sourceAppId },
				).catch((error: unknown) => {
					console.warn("Couldn't read the app's packages for the fork:", error);
					return null;
				});
				const sinksByEvent = await loadLocalSinks(
					backend.sinkState,
					sourceAppId,
				);
				const sourceRoles = await backend.roleState
					.getRoles(sourceAppId)
					.catch(() => [undefined, []] as [string | undefined, IBackendRole[]]);
				const skipped: string[] = [];
				const needsReauth: string[] = [];

				const contentUpload = await uploadLocalContent(
					backend.apiState,
					profile,
					sourceAppId,
					begin.new_app_id,
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
						widget: normalizeWidgetForPersistence(stripForkSecrets(widget)),
						metadata: metadata ? stripForkSecrets(metadata) : undefined,
					});
				}

				const preparedPages: IPage[] = [];
				for (const page of pageList) {
					preparedPages.push(
						normalizePageForPersistence(
							stripForkSecrets(
								await backend.pageState.getPage(
									sourceAppId,
									page.pageId,
									page.boardId,
								),
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
					// The destination's sink row is derived from the event on
					// upsert and runs as the forking user. OAuth grants are
					// provider-bound and always need a fresh consent, so they
					// are reported instead of copied — same rule the online →
					// online fork applies.
					const sink = sinksByEvent.get(event.id);
					const oauthProviders = sink?.oauth_providers ?? [];
					if (oauthProviders.length > 0) {
						needsReauth.push(event.name ?? event.id);
					}
					await backend.apiState.put<IEvent>(
						profile,
						`apps/${begin.new_app_id}/events/${preparedEvent.id}`,
						{
							event: preparedEvent,
							profile_id: profile.id,
						},
					);
				}

				if (!localPackages) {
					skipped.push("packages (couldn't read this app's packages)");
				}
				for (const [packageId, version] of Object.entries(
					localPackages ?? {},
				)) {
					try {
						await backend.apiState.post<unknown>(
							profile,
							`apps/${begin.new_app_id}/packages`,
							{ packageId, version, autoUpdate: false },
						);
					} catch (error) {
						console.warn("Skipping package during fork:", packageId, error);
						skipped.push(`package ${packageId} (${packageSkipReason(error)})`);
					}
				}

				const [sourceDefaultRoleId, roles] = sourceRoles;
				for (const role of roles) {
					if (role.id === sourceDefaultRoleId) continue;
					if (SYSTEM_ROLE_NAMES.has(role.name)) continue;
					try {
						await backend.roleState.upsertRole(begin.new_app_id, {
							...role,
							app_id: begin.new_app_id,
						});
					} catch (error) {
						console.warn("Skipping role during fork:", role.name, error);
						skipped.push(`role ${role.name}`);
					}
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

				await backend.apiState.put<void>(
					profile,
					`apps/${begin.new_app_id}/meta?language=${LANGUAGE}`,
					stripForkSecrets(appMeta),
				);

				const finalized =
					await backend.apiState.post<IFinalizeOnlineForkResponse>(
						profile,
						`apps/${begin.new_app_id}/fork/online/finalize`,
						{
							visibility: "private",
							app_settings: getFinalizeAppSettings(sourceApp),
						},
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
					`Online fork created (${contentUpload.files_copied} content files, ${formatBytes(
						finalized.total_size_bytes,
					)})`,
					{ id: toastId },
				);
				if (skipped.length > 0) {
					toast.warning(`Not copied: ${skipped.join(", ")}`);
				}
				if (needsReauth.length > 0) {
					toast.warning(
						`Reconnect the accounts for: ${needsReauth.join(", ")}`,
					);
				}
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
