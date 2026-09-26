import type { TFunction } from "i18next";
import { schemaFields } from "../../../lib/app-build/table-schema-readback";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { humanFileSize } from "../../../lib/utils";
import type {
	IDatabaseVersion,
	IIndexConfig,
} from "../../../state/backend-state/db-state";
import type {
	IOfflineOperation,
	IOfflineOperationLookup,
	IOfflineResourceLabel,
	IOfflineTableState,
	OfflineOperationKind,
	OfflineOperationLookupState,
	OfflineSyncState,
	OfflineTablePurpose,
} from "../../../state/backend-state/offline-writes-state";

export type OfflineTranslate = TFunction<"settings">;

export const DEFAULT_KEY_COLUMN = "id";
export const OPERATIONS_PAGE_SIZE = 50;
export const SKIP_REASON_MAX_LENGTH = 1024;
export const MEBIBYTE = 1024 * 1024;
export const DAY_SECONDS = 24 * 3600;
const WEEK_MS = 7 * DAY_SECONDS * 1000;

/** Device-local queue state must never be restored from the persisted query cache. */
export const LIVE_ONLY = { persist: false } as const;

export const offlineQueryKeys = {
	all: (appId: string) => ["offline-writes", appId] as const,
	overview: (appId: string) => ["offline-writes", appId, "overview"] as const,
	operations: (appId: string) =>
		["offline-writes", appId, "operations"] as const,
};

const TABLE_NAME_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;
const KEY_COLUMN_PATTERN = /^[A-Za-z0-9_]{1,128}$/;
const KEY_COLUMN_TYPES = new Set([
	"utf8",
	"largeutf8",
	"int8",
	"int16",
	"int32",
	"int64",
	"uint8",
	"uint16",
	"uint32",
	"uint64",
]);
const KEY_INDEX_TYPES = new Set(["BTREE", "BITMAP"]);

export function isSupportedTableName(table: string): boolean {
	return TABLE_NAME_PATTERN.test(table);
}

export function keyColumnCandidates(schema: unknown): string[] {
	return schemaFields(schema).flatMap((field) => {
		const name = field.name;
		const type = field.data_type ?? field.type;
		return typeof name === "string" &&
			KEY_COLUMN_PATTERN.test(name) &&
			typeof type === "string" &&
			KEY_COLUMN_TYPES.has(type.toLowerCase())
			? [name]
			: [];
	});
}

export function defaultKeyColumn(candidates: string[]): string | undefined {
	return candidates.includes(DEFAULT_KEY_COLUMN)
		? DEFAULT_KEY_COLUMN
		: candidates[0];
}

export function hasKeyIndex(indices: IIndexConfig[], key: string): boolean {
	return indices.some(
		(index) =>
			index.columns.length === 1 &&
			index.columns[0] === key &&
			KEY_INDEX_TYPES.has(
				index.index_type.toUpperCase().replace(/[^A-Z]/g, ""),
			),
	);
}

export function recentVersionCount(
	versions: IDatabaseVersion[],
	nowMs: number,
): number {
	return versions.filter((version) => {
		const at = Date.parse(version.timestamp);
		return Number.isFinite(at) && nowMs - at <= WEEK_MS;
	}).length;
}

/** The §3.2 replay permission matrix, inverted. */
export function isReadOnlyForTable(
	purpose: OfflineTablePurpose,
	can: (...permissions: RolePermissions[]) => boolean,
): boolean {
	const writes = can(RolePermissions.WriteFiles, RolePermissions.WriteDatabase);
	return purpose === "storage"
		? !writes
		: !writes && !can(RolePermissions.ExecuteEvents);
}

export function isRegistered(table: IOfflineTableState): boolean {
	return table.status === "settling" || table.status === "ready";
}

export interface HeadActions {
	retry: boolean;
	skip: boolean;
	keepBoth: boolean;
}

export function headActions(operation: IOfflineOperation): HeadActions {
	return {
		retry:
			operation.state === "blocked" || operation.state === "outcome_unknown",
		skip: true,
		keepBoth:
			operation.state === "conflict" &&
			operation.kind === "fileWrite" &&
			operation.resource.type === "file",
	};
}

export function headMessage(
	t: OfflineTranslate,
	operation: IOfflineOperation,
): string | undefined {
	switch (operation.state) {
		case "conflict":
			return operation.resource.type === "file"
				? t(
						"settings:offlineAccess.headFileConflict",
						"The cloud already has a different file at this path. Keep both to upload this device's version under a new name, or skip it.",
					)
				: t(
						"settings:offlineAccess.headConflict",
						"The cloud copy changed after this device made this change. Skip it to continue; the change will not be applied.",
					);
		case "outcome_unknown":
			return t(
				"settings:offlineAccess.headUnknown",
				"The hub may already have applied this change. Retry to check again, or skip it.",
			);
		case "blocked":
			return blockedMessage(t, operation);
		default:
			return undefined;
	}
}

function blockedMessage(
	t: OfflineTranslate,
	operation: IOfflineOperation,
): string {
	switch (operation.errorCode) {
		case "hub_limit":
			return t(
				"settings:offlineAccess.headHubLimit",
				"This change is larger than the hub accepts. Ask the hub operator to raise the limit, then retry, or skip it.",
			);
		case "forbidden":
			return t(
				"settings:offlineAccess.headForbidden",
				"Your role in this project no longer allows this change. Ask a project admin for write access, then retry, or skip it.",
			);
		case "endpoint_missing":
			return t(
				"settings:offlineAccess.headEndpointMissing",
				"This hub does not accept offline changes from the desktop app yet. Retry after the hub is updated.",
			);
		case "subject_mismatch":
			return t(
				"settings:offlineAccess.headSubjectMismatch",
				"This change was queued for another account. Sign in as that account or update the automation's token, then retry.",
			);
		default:
			return t(
				"settings:offlineAccess.headBlocked",
				"The hub did not accept this change: {{message}}",
				{ message: operation.error ?? "" },
			);
	}
}

export function availabilityLine(
	t: OfflineTranslate,
	table: IOfflineTableState,
): string {
	if (table.offlineComplete) {
		return table.prefetch
			? t(
					"settings:offlineAccess.availabilityComplete",
					"Fully available offline",
				)
			: t(
					"settings:offlineAccess.availabilityCompleteUnpinned",
					"Everything is on this device for now, but it can be removed to free space. Turn on Download everything to keep it.",
				);
	}
	const cached = humanFileSize(table.cachedBytes);
	const total =
		table.totalBytes == null ? undefined : humanFileSize(table.totalBytes);
	if (table.downloading) {
		return total
			? t(
					"settings:offlineAccess.availabilityDownloading",
					"Downloading… {{cached}} of {{total}}",
					{ cached, total },
				)
			: t(
					"settings:offlineAccess.availabilityDownloadingUnknown",
					"Downloading… {{cached}} so far",
					{ cached },
				);
	}
	return total
		? t(
				"settings:offlineAccess.availabilityPartial",
				"{{cached}} of {{total}} on this device. Offline, flows can only use data that is already here.",
				{ cached, total },
			)
		: t(
				"settings:offlineAccess.availabilityPartialUnknown",
				"{{cached}} on this device. Offline, flows can only use data that is already here.",
				{ cached },
			);
}

export function tableStatusLabel(
	t: OfflineTranslate,
	table: IOfflineTableState,
): string {
	if (table.remoteMissing) {
		return t(
			"settings:offlineAccess.statusRemoteMissing",
			"Deleted in the cloud",
		);
	}
	switch (table.status) {
		case "preparing":
			return t("settings:offlineAccess.statusPreparing", "Preparing…");
		case "settling":
			return t("settings:offlineAccess.statusSettling", "Finishing setup…");
		case "ready":
			return t("settings:offlineAccess.statusReady", "Ready");
		case "error":
			return t("settings:offlineAccess.statusError", "Setup failed");
	}
}

export function formatTimestamp(seconds: number): string {
	return new Date(seconds * 1000).toLocaleString();
}

/** Secondary lines under a configured table's status, most important first. */
export function tableDetailLines(
	t: OfflineTranslate,
	table: IOfflineTableState,
): string[] {
	const lines: string[] = [];
	if (table.error) lines.push(table.error);
	if (table.status === "settling") {
		if (table.waitingForRuns > 0) {
			lines.push(
				t("settings:offlineAccess.waitingForRuns", {
					count: table.waitingForRuns,
					defaultValue_one:
						"Waiting for {{count}} running flow of this project to finish",
					defaultValue_other:
						"Waiting for {{count}} running flows of this project to finish",
				}),
			);
		}
		if (table.mirrorError) {
			lines.push(
				t(
					"settings:offlineAccess.settlingRetry",
					"Setup continues when the hub is reachable: {{reason}}",
					{ reason: table.mirrorError },
				),
			);
		}
		return lines;
	}
	const time =
		table.refreshedAt == null ? undefined : formatTimestamp(table.refreshedAt);
	if (table.status === "ready" && table.mirrorError && !table.remoteMissing) {
		lines.push(
			t(
				"settings:offlineAccess.usingCopyFrom",
				"Using the copy from {{time}}: {{reason}}",
				{
					time: time ?? t("common:unknown", "Unknown"),
					reason: table.mirrorError,
				},
			),
		);
	} else if (time) {
		lines.push(
			t("settings:offlineAccess.refreshedAt", "Updated {{time}}", { time }),
		);
	}
	return lines;
}

export function syncStateLabel(
	t: OfflineTranslate,
	state: OfflineSyncState,
	account?: string | null,
): string {
	switch (state) {
		case "idle":
			return t("settings:offlineAccess.syncIdle", "Synced");
		case "syncing":
			return t("settings:offlineAccess.syncSyncing", "Syncing…");
		case "waitingForConnection":
			return t(
				"settings:offlineAccess.syncWaitingForConnection",
				"Waiting for a connection to the hub",
			);
		case "waitingForSignIn":
			return t(
				"settings:offlineAccess.syncWaitingForSignIn",
				"Sign in as {{account}} to sync these changes",
				{ account: account ?? "" },
			);
		case "hubError":
			return t(
				"settings:offlineAccess.syncHubError",
				"The hub could not accept changes right now. Retrying…",
			);
		case "blocked":
			return t(
				"settings:offlineAccess.syncBlocked",
				"Syncing stopped at a change that needs your decision",
			);
	}
}

export function operationKindLabel(
	t: OfflineTranslate,
	kind: OfflineOperationKind,
): string {
	switch (kind) {
		case "tableInsert":
			return t("settings:offlineAccess.kindTableInsert", "Add rows");
		case "tableUpsert":
			return t("settings:offlineAccess.kindTableUpsert", "Add or replace rows");
		case "tableUpdate":
			return t("settings:offlineAccess.kindTableUpdate", "Update rows");
		case "tableDelete":
			return t("settings:offlineAccess.kindTableDelete", "Delete rows");
		case "fileWrite":
			return t("settings:offlineAccess.kindFileWrite", "Save file");
		case "fileDelete":
			return t("settings:offlineAccess.kindFileDelete", "Delete file");
	}
}

export function resourceName(resource: IOfflineResourceLabel): string {
	return resource.type === "table" ? resource.table : resource.path;
}

export function operationLookupState(
	operation: IOfflineOperation,
): OfflineOperationLookupState {
	return operation.state === "outcome_unknown"
		? "outcomeUnknown"
		: operation.state;
}

export function lookupLabel(
	t: OfflineTranslate,
	lookup: IOfflineOperationLookup,
): string {
	switch (lookup.state) {
		case "applied":
			return t("settings:offlineAccess.lookupApplied", "Synced");
		case "pending":
		case "attempting":
			return t("settings:offlineAccess.lookupPending", "Waiting to sync");
		case "blocked":
		case "conflict":
		case "outcomeUnknown":
			return t(
				"settings:offlineAccess.syncBlocked",
				"Syncing stopped at a change that needs your decision",
			);
		case "skipped":
			return t("settings:offlineAccess.lookupSkipped", "Skipped");
		case "superseded":
			return t(
				"settings:offlineAccess.lookupSuperseded",
				"Combined into change {{id}}",
				{ id: lookup.supersededBy ?? "" },
			);
		case "unknown":
			return t(
				"settings:offlineAccess.lookupUnknown",
				"This device has no record of this change",
			);
	}
}

export function canSubmitSkip(
	reason: string,
	attempts: number,
	acknowledged: boolean,
): boolean {
	const trimmed = reason.trim();
	return (
		trimmed.length > 0 &&
		trimmed.length <= SKIP_REASON_MAX_LENGTH &&
		(attempts === 0 || acknowledged)
	);
}

export function mirrorLimitError(
	t: OfflineTranslate,
	maxMirrorBytes: number,
	requiredBytes: number,
): string | undefined {
	return maxMirrorBytes < requiredBytes
		? t(
				"settings:offlineAccess.mirrorLimitTooLow",
				"Tables that download everything and tables with queued changes need at least {{size}}.",
				{ size: humanFileSize(requiredBytes) },
			)
		: undefined;
}

export interface OfflineTableRow {
	purpose: OfflineTablePurpose;
	table: string;
	state?: IOfflineTableState;
}

/** Listed tables plus configured ones the listing lacks, e.g. because it failed offline. */
export function mergeTableRows(
	purpose: OfflineTablePurpose,
	listed: readonly string[] | undefined,
	configured: readonly IOfflineTableState[],
): OfflineTableRow[] {
	const states = new Map(
		configured
			.filter((table) => table.purpose === purpose)
			.map((table) => [table.table, table]),
	);
	const names = new Set([...(listed ?? []), ...states.keys()]);
	return [...names]
		.sort((a, b) => a.localeCompare(b))
		.map((table) => ({ purpose, table, state: states.get(table) }));
}
