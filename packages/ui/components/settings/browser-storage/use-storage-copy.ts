"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import type {
	BrowserStorageCategory,
	BrowserStoreId,
} from "../../../lib/browser-storage/inventory";
import type { RetentionKind } from "../../../lib/browser-storage/retention";

interface Copy {
	readonly label: string;
	readonly description: string;
}

export interface StorageCopy {
	readonly categories: Readonly<Record<BrowserStorageCategory, Copy>>;
	readonly stores: Readonly<Record<BrowserStoreId, Copy>>;
	readonly retention: Readonly<Record<RetentionKind, string>>;
	readonly deleteWarnings: Readonly<
		Partial<Record<BrowserStorageCategory, string>>
	>;
}

export function useStorageCopy(): StorageCopy {
	const { t } = useTranslation("settings");
	return useMemo(
		() => ({
			categories: {
				boards: {
					label: t("storageCategoryBoards", "Board history"),
					description: t(
						"storageCategoryBoardsDescription",
						"Undo steps and the snapshot that shows what changed since your last visit, per board.",
					),
				},
				activity: {
					label: t("storageCategoryActivity", "Chats & activity"),
					description: t(
						"storageCategoryActivityDescription",
						"Conversations, FlowPilot sessions, form runs and notifications kept in this browser.",
					),
				},
				caches: {
					label: t("storageCategoryCaches", "Caches"),
					description: t(
						"storageCategoryCachesDescription",
						"Copies of server data and rendered pages that make the app faster. They are rebuilt when needed.",
					),
				},
				local: {
					label: t("storageCategoryLocal", "Saved in this browser"),
					description: t(
						"storageCategoryLocalDescription",
						"Values and preferences that exist only here. Deleting them cannot be undone.",
					),
				},
				protected: {
					label: t("storageCategoryProtected", "Protected"),
					description: t(
						"storageCategoryProtectedDescription",
						"Sign-in tokens, device keys and changes waiting to sync. The app manages these; they are never deleted here.",
					),
				},
				other: {
					label: t("storageCategoryOther", "Other"),
					description: t(
						"storageCategoryOtherDescription",
						"Storage of libraries and browser overhead that belongs to no single store.",
					),
				},
			},
			stores: {
				chats: {
					label: t("storageStoreChats", "App chats"),
					description: t(
						"storageStoreChatsDescription",
						"Chat conversations in your apps. Pinned conversations are kept.",
					),
				},
				globalChat: {
					label: t("storageStoreGlobalChat", "Assistant conversations"),
					description: t(
						"storageStoreGlobalChatDescription",
						"Conversations with the Flow-Like assistant. Pinned conversations are kept.",
					),
				},
				flowpilot: {
					label: t("storageStoreFlowpilot", "FlowPilot sessions"),
					description: t(
						"storageStoreFlowpilotDescription",
						"FlowPilot conversations about boards and pages. Pinned sessions are kept.",
					),
				},
				formRuns: {
					label: t("storageStoreFormRuns", "Form run history"),
					description: t(
						"storageStoreFormRunsDescription",
						"Earlier submissions that forms remember. Saved presets are kept.",
					),
				},
				discardedSync: {
					label: t("storageStoreDiscardedSync", "Discarded offline changes"),
					description: t(
						"storageStoreDiscardedSyncDescription",
						"Edits that could not be synced and were set aside for recovery.",
					),
				},
				notifications: {
					label: t("storageStoreNotifications", "Notifications"),
					description: t(
						"storageStoreNotificationsDescription",
						"Notifications stored in this browser.",
					),
				},
				queryCache: {
					label: t("storageStoreQueryCache", "Data cache"),
					description: t(
						"storageStoreQueryCacheDescription",
						"Recent server responses, so pages open instantly.",
					),
				},
				pageSurfaces: {
					label: t("storageStorePageSurfaces", "Page previews"),
					description: t(
						"storageStorePageSurfacesDescription",
						"The last rendered version of app pages, shown while they load.",
					),
				},
				viewports: {
					label: t("storageStoreViewports", "Board positions"),
					description: t(
						"storageStoreViewportsDescription",
						"Where you last left the canvas of each board.",
					),
				},
				temporaryFiles: {
					label: t("storageStoreTemporaryFiles", "Temporary files"),
					description: t(
						"storageStoreTemporaryFilesDescription",
						"Files staged for uploads and runs.",
					),
				},
				nodeSuggestions: {
					label: t("storageStoreNodeSuggestions", "Node suggestions"),
					description: t(
						"storageStoreNodeSuggestionsDescription",
						"What the editor learned about the nodes you use. It is learned again after deleting.",
					),
				},
				keyval: {
					label: t("storageStoreKeyval", "General cache"),
					description: t(
						"storageStoreKeyvalDescription",
						"Small cached lists and navigation state.",
					),
				},
				cacheStorage: {
					label: t("storageStoreCacheStorage", "Cached files"),
					description: t(
						"storageStoreCacheStorageDescription",
						"Files kept in the browser's Cache Storage.",
					),
				},
				runtimeVariables: {
					label: t("storageStoreRuntimeVariables", "Runtime variable values"),
					description: t(
						"storageStoreRuntimeVariablesDescription",
						"Values you entered for runtime variables. Apps ask for them again after deleting.",
					),
				},
				pageState: {
					label: t("storageStorePageState", "Page data"),
					description: t(
						"storageStorePageStateDescription",
						"Inputs and state that app pages keep between visits.",
					),
				},
				formPresets: {
					label: t("storageStoreFormPresets", "Form presets"),
					description: t(
						"storageStoreFormPresetsDescription",
						"Presets and preferences you saved in forms.",
					),
				},
				lanceSettings: {
					label: t("storageStoreLanceSettings", "Table explorer settings"),
					description: t(
						"storageStoreLanceSettingsDescription",
						"Column and view settings of the data explorer.",
					),
				},
				appShortcuts: {
					label: t("storageStoreAppShortcuts", "App shortcuts"),
					description: t(
						"storageStoreAppShortcutsDescription",
						"Pinned apps and visibility preferences.",
					),
				},
				preferences: {
					label: t("storageStorePreferences", "Preferences & session"),
					description: t(
						"storageStorePreferencesDescription",
						"Local settings and your sign-in session.",
					),
				},
				oauth: {
					label: t("storageStoreOauth", "Connected accounts"),
					description: t(
						"storageStoreOauthDescription",
						"Tokens of services you connected. Disconnect them where you connected them.",
					),
				},
				deviceVault: {
					label: t("storageStoreDeviceVault", "Device keys"),
					description: t(
						"storageStoreDeviceVaultDescription",
						"Encrypted keys for managing your devices.",
					),
				},
				pendingSync: {
					label: t("storageStorePendingSync", "Changes waiting to sync"),
					description: t(
						"storageStorePendingSyncDescription",
						"Board edits that have not reached the server yet.",
					),
				},
			},
			retention: {
				undoHistory: t("storageRetentionUndoHistory", "Undo history"),
				boardSnapshots: t(
					"storageRetentionBoardSnapshots",
					"Board change snapshots",
				),
				chats: t("storageRetentionChats", "App chats"),
				globalChat: t("storageRetentionGlobalChat", "Assistant conversations"),
				flowpilot: t("storageRetentionFlowpilot", "FlowPilot sessions"),
				discardedSync: t(
					"storageRetentionDiscardedSync",
					"Discarded offline changes",
				),
				temporaryFiles: t("storageRetentionTemporaryFiles", "Temporary files"),
				viewports: t("storageRetentionViewports", "Board positions"),
			},
			deleteWarnings: {
				boards: t(
					"storageDeleteWarningBoards",
					"You can no longer undo earlier edits on these boards. The boards themselves are not changed.",
				),
				activity: t(
					"storageDeleteWarningActivity",
					"Pinned conversations are kept.",
				),
				caches: t(
					"storageDeleteWarningCaches",
					"Caches are rebuilt the next time they are needed.",
				),
				local: t(
					"storageDeleteWarningLocal",
					"This data exists only in this browser and cannot be recovered.",
				),
			},
		}),
		[t],
	);
}
