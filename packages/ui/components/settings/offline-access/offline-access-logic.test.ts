import { describe, expect, test } from "bun:test";
import { i18n } from "@flow-like/locales";
import { RolePermissions } from "../../../lib/permission/role-permission";
import type {
	IOfflineOperation,
	IOfflineTableState,
} from "../../../state/backend-state/offline-writes-state";
import {
	type OfflineTranslate,
	availabilityLine,
	canSubmitSkip,
	defaultKeyColumn,
	formatTimestamp,
	hasKeyIndex,
	headActions,
	headMessage,
	isReadOnlyForTable,
	isSupportedTableName,
	keyColumnCandidates,
	lookupLabel,
	mergeTableRows,
	mirrorLimitError,
	recentVersionCount,
	tableDetailLines,
	tableStatusLabel,
} from "./offline-access-logic";

const t = i18n.getFixedT("en", "settings") as OfflineTranslate;
const MiB = 1024 * 1024;

function table(
	overrides: Partial<IOfflineTableState> = {},
): IOfflineTableState {
	return {
		purpose: "storage",
		table: "orders",
		primaryKey: "id",
		prefetch: false,
		status: "ready",
		pendingCount: 0,
		cachedBytes: 2 * MiB,
		totalBytes: 8 * MiB,
		localBytes: 0,
		offlineComplete: false,
		downloading: false,
		waitingForRuns: 0,
		remoteMissing: false,
		...overrides,
	};
}

function operation(
	overrides: Partial<IOfflineOperation> = {},
): IOfflineOperation {
	return {
		sequence: 1,
		operationId: "op-1",
		kind: "tableInsert",
		resource: { type: "table", purpose: "storage", table: "orders" },
		state: "blocked",
		attempts: 0,
		createdAt: 1_700_000_000,
		bytes: 10,
		...overrides,
	};
}

const holding =
	(...held: RolePermissions[]) =>
	(...asked: RolePermissions[]) =>
		asked.some((permission) => held.some((h) => h.equals(permission)));

describe("key column", () => {
	const schema = {
		fields: [
			{ name: "id", data_type: "Utf8" },
			{ name: "big", data_type: "LargeUtf8" },
			{ name: "n", data_type: "Int64" },
			{ name: "u", data_type: "UInt8" },
			{ name: "price", data_type: "Float64" },
			{ name: "flag", data_type: "Boolean" },
			{ name: "vector", data_type: { FixedSizeList: [{}, 3] } },
			{ name: "has space", data_type: "Utf8" },
			{ name: "dotted.name", data_type: "Utf8" },
		],
	};

	test("offers only text and whole-number columns with plain names", () => {
		expect(keyColumnCandidates(schema)).toEqual(["id", "big", "n", "u"]);
		expect(keyColumnCandidates(undefined)).toEqual([]);
	});

	test("defaults to id, else the first candidate", () => {
		expect(defaultKeyColumn(["n", "id"])).toBe("id");
		expect(defaultKeyColumn(["n", "u"])).toBe("n");
		expect(defaultKeyColumn([])).toBeUndefined();
	});

	test("wants a BTree or Bitmap index on exactly the key column", () => {
		expect(
			hasKeyIndex([{ name: "i", index_type: "BTREE", columns: ["id"] }], "id"),
		).toBe(true);
		expect(
			hasKeyIndex([{ name: "i", index_type: "Bitmap", columns: ["id"] }], "id"),
		).toBe(true);
		expect(
			hasKeyIndex([{ name: "i", index_type: "FTS", columns: ["id"] }], "id"),
		).toBe(false);
		expect(
			hasKeyIndex(
				[{ name: "i", index_type: "BTREE", columns: ["id", "other"] }],
				"id",
			),
		).toBe(false);
		expect(
			hasKeyIndex([{ name: "i", index_type: "BTREE", columns: ["n"] }], "id"),
		).toBe(false);
	});
});

test("table names follow the engine's rule", () => {
	expect(isSupportedTableName("orders_2024-v1")).toBe(true);
	expect(isSupportedTableName("orders.v1")).toBe(false);
	expect(isSupportedTableName("orders v1")).toBe(false);
	expect(isSupportedTableName("")).toBe(false);
	expect(isSupportedTableName("a".repeat(129))).toBe(false);
});

test("counts versions of the last 7 days", () => {
	const now = Date.parse("2026-09-24T12:00:00Z");
	expect(
		recentVersionCount(
			[
				{ version: 1, timestamp: "2026-09-01T00:00:00Z", metadata: {} },
				{ version: 2, timestamp: "2026-09-20T00:00:00Z", metadata: {} },
				{ version: 3, timestamp: "2026-09-24T11:00:00Z", metadata: {} },
				{ version: 4, timestamp: "not a date", metadata: {} },
			],
			now,
		),
	).toBe(2);
});

describe("read-only warning", () => {
	test("project tables need WriteFiles or WriteDatabase", () => {
		expect(
			isReadOnlyForTable("storage", holding(RolePermissions.ExecuteEvents)),
		).toBe(true);
		expect(
			isReadOnlyForTable("storage", holding(RolePermissions.WriteDatabase)),
		).toBe(false);
		expect(
			isReadOnlyForTable("storage", holding(RolePermissions.WriteFiles)),
		).toBe(false);
	});

	test("user tables also accept ExecuteEvents", () => {
		expect(
			isReadOnlyForTable("user", holding(RolePermissions.ExecuteEvents)),
		).toBe(false);
		expect(isReadOnlyForTable("user", holding(RolePermissions.ReadFiles))).toBe(
			true,
		);
	});
});

describe("queue heads", () => {
	test("retry only for blocked and uncertain heads, skip always", () => {
		expect(headActions(operation({ state: "blocked" }))).toEqual({
			retry: true,
			skip: true,
			keepBoth: false,
		});
		expect(headActions(operation({ state: "outcome_unknown" })).retry).toBe(
			true,
		);
		expect(headActions(operation({ state: "conflict" }))).toEqual({
			retry: false,
			skip: true,
			keepBoth: false,
		});
	});

	test("keep both only for conflicting file writes", () => {
		const file = {
			type: "file" as const,
			purpose: "files" as const,
			path: "apps/a/upload/x.csv",
		};
		expect(
			headActions(
				operation({ state: "conflict", kind: "fileWrite", resource: file }),
			).keepBoth,
		).toBe(true);
		expect(
			headActions(
				operation({ state: "conflict", kind: "fileDelete", resource: file }),
			).keepBoth,
		).toBe(false);
		expect(
			headActions(
				operation({ state: "blocked", kind: "fileWrite", resource: file }),
			).keepBoth,
		).toBe(false);
	});

	test("messages follow the error code", () => {
		const blocked = (errorCode: IOfflineOperation["errorCode"]) =>
			headMessage(t, operation({ errorCode, error: "raw server text" }));
		expect(blocked("hub_limit")).toContain("larger than the hub accepts");
		expect(blocked("forbidden")).toContain("no longer allows this change");
		expect(blocked("endpoint_missing")).toContain(
			"does not accept offline changes",
		);
		expect(blocked("subject_mismatch")).toContain("queued for another account");
		expect(blocked("invalid")).toBe(
			"The hub did not accept this change: raw server text",
		);
		expect(blocked("digest_reused")).toBe(
			"The hub did not accept this change: raw server text",
		);
		expect(headMessage(t, operation({ state: "conflict" }))).toContain(
			"The cloud copy changed",
		);
		expect(
			headMessage(
				t,
				operation({
					state: "conflict",
					resource: { type: "file", purpose: "user", path: "a.txt" },
				}),
			),
		).toContain("different file at this path");
		expect(headMessage(t, operation({ state: "outcome_unknown" }))).toContain(
			"may already have applied",
		);
		expect(headMessage(t, operation({ state: "pending" }))).toBeUndefined();
	});

	test("skipping needs a reason, and an acknowledgement once attempted", () => {
		expect(canSubmitSkip("  ", 0, false)).toBe(false);
		expect(canSubmitSkip("duplicate", 0, false)).toBe(true);
		expect(canSubmitSkip("duplicate", 2, false)).toBe(false);
		expect(canSubmitSkip("duplicate", 2, true)).toBe(true);
		expect(canSubmitSkip("x".repeat(1025), 0, false)).toBe(false);
	});
});

describe("availability", () => {
	test("fully available only with Download everything", () => {
		expect(
			availabilityLine(t, table({ offlineComplete: true, prefetch: true })),
		).toBe("Fully available offline");
		expect(
			availabilityLine(t, table({ offlineComplete: true, prefetch: false })),
		).toContain("it can be removed to free space");
	});

	test("downloading and partial lines, with and without a known total", () => {
		expect(availabilityLine(t, table({ downloading: true }))).toBe(
			"Downloading… 2.0 MiB of 8.0 MiB",
		);
		expect(
			availabilityLine(t, table({ downloading: true, totalBytes: null })),
		).toBe("Downloading… 2.0 MiB so far");
		expect(availabilityLine(t, table())).toBe(
			"2.0 MiB of 8.0 MiB on this device. Offline, flows can only use data that is already here.",
		);
		expect(availabilityLine(t, table({ totalBytes: null }))).toBe(
			"2.0 MiB on this device. Offline, flows can only use data that is already here.",
		);
	});
});

describe("table status", () => {
	test("labels, with remote deletion first", () => {
		expect(tableStatusLabel(t, table({ status: "preparing" }))).toBe(
			"Preparing…",
		);
		expect(tableStatusLabel(t, table({ status: "settling" }))).toBe(
			"Finishing setup…",
		);
		expect(
			tableStatusLabel(t, table({ status: "error", remoteMissing: true })),
		).toBe("Deleted in the cloud");
	});

	test("settling rows name waiting runs and the activation retry reason", () => {
		expect(
			tableDetailLines(
				t,
				table({
					status: "settling",
					waitingForRuns: 2,
					mirrorError: "The cloud table could not be read: timeout",
				}),
			),
		).toEqual([
			"Waiting for 2 running flows of this project to finish",
			"Setup continues when the hub is reachable: The cloud table could not be read: timeout",
		]);
	});

	test("ready rows with a mirror error use the older copy", () => {
		const refreshedAt = 1_700_000_000;
		expect(
			tableDetailLines(
				t,
				table({
					refreshedAt,
					mirrorError: "Could not refresh from the cloud: offline",
				}),
			),
		).toEqual([
			`Using the copy from ${formatTimestamp(refreshedAt)}: Could not refresh from the cloud: offline`,
		]);
		expect(tableDetailLines(t, table({ refreshedAt }))).toEqual([
			`Updated ${formatTimestamp(refreshedAt)}`,
		]);
	});

	test("remote deletion does not claim an older copy", () => {
		const lines = tableDetailLines(
			t,
			table({
				status: "error",
				remoteMissing: true,
				error: "Table 'orders' was deleted in the cloud.",
				mirrorError: "gone",
				refreshedAt: 1_700_000_000,
			}),
		);
		expect(lines[0]).toBe("Table 'orders' was deleted in the cloud.");
		expect(lines.some((line) => line.startsWith("Using the copy"))).toBe(false);
	});
});

test("lookup labels", () => {
	expect(lookupLabel(t, { state: "applied" })).toBe("Synced");
	expect(lookupLabel(t, { state: "attempting" })).toBe("Waiting to sync");
	expect(lookupLabel(t, { state: "superseded", supersededBy: "op-9" })).toBe(
		"Combined into change op-9",
	);
	expect(lookupLabel(t, { state: "unknown" })).toBe(
		"This device has no record of this change",
	);
});

test("the mirror limit cannot drop below the required size", () => {
	expect(mirrorLimitError(t, 100 * MiB, 200 * MiB)).toBe(
		"Tables that download everything and tables with queued changes need at least 200.0 MiB.",
	);
	expect(mirrorLimitError(t, 200 * MiB, 200 * MiB)).toBeUndefined();
});

test("configured tables stay listed when the table list is unavailable", () => {
	const configured = [
		table({ table: "orders" }),
		table({ purpose: "user", table: "notes" }),
	];
	expect(
		mergeTableRows("storage", ["zeta", "orders"], configured).map((row) => [
			row.table,
			!!row.state,
		]),
	).toEqual([
		["orders", true],
		["zeta", false],
	]);
	expect(
		mergeTableRows("user", undefined, configured).map((row) => row.table),
	).toEqual(["notes"]);
});
