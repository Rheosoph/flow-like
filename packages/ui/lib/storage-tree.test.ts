import { describe, expect, test } from "bun:test";
import type { IStorageItem } from "./schema/storage/storage-item";
import {
	childPrefix,
	normalizeStorageLocation,
	parentPrefix,
	parseStorageNodeId,
	sortStorageEntries,
	storageItemName,
	storageNodeId,
	storagePrefixTrail,
	storageTreeEntry,
} from "./storage-tree";

const APP_BASE = "apps/app-1/upload";
const USER_BASE = "users/sub-1/apps/app-1";

const item = (location: string, isDir = false): IStorageItem => ({
	location,
	last_modified: "",
	size: 0,
	is_dir: isDir,
});

describe("normalizeStorageLocation", () => {
	test("keeps desktop's already-relative paths as they are", () => {
		expect(normalizeStorageLocation("a.pdf", "")).toBe("a.pdf");
		expect(normalizeStorageLocation("docs", "")).toBe("docs");
		expect(normalizeStorageLocation("docs/a.pdf", "docs")).toBe("docs/a.pdf");
		expect(normalizeStorageLocation("docs/2026/q1/a.pdf", "docs/2026/q1")).toBe(
			"docs/2026/q1/a.pdf",
		);
	});

	test("strips the base off cloud's absolute object-store keys", () => {
		expect(normalizeStorageLocation(`${APP_BASE}/a.pdf`, "")).toBe("a.pdf");
		expect(normalizeStorageLocation(`${APP_BASE}/docs`, "")).toBe("docs");
		expect(normalizeStorageLocation(`${APP_BASE}/docs/a.pdf`, "docs")).toBe(
			"docs/a.pdf",
		);
		expect(
			normalizeStorageLocation(
				`${APP_BASE}/docs/2026/q1/a.pdf`,
				"docs/2026/q1",
			),
		).toBe("docs/2026/q1/a.pdf");
	});

	test("strips the user-storage base the same way", () => {
		expect(normalizeStorageLocation(`${USER_BASE}/a.pdf`, "")).toBe("a.pdf");
		expect(
			normalizeStorageLocation(`${USER_BASE}/private/a.pdf`, "private"),
		).toBe("private/a.pdf");
	});

	// construct_upload/construct_user_upload fold the prefix onto the base with no
	// absolute-key stripping, so an unnormalized location listed again resolves to
	// apps/<id>/upload/apps/<id>/upload/... — an always-empty folder.
	test("never re-prefixes a key that already carries the base", () => {
		const folder = normalizeStorageLocation(`${APP_BASE}/docs`, "");
		expect(folder).toBe("docs");

		const child = normalizeStorageLocation(`${APP_BASE}/docs/a.pdf`, folder);
		expect(child).toBe("docs/a.pdf");
		expect(child).not.toContain(APP_BASE);

		// What the raw location does when fed back in as a prefix.
		expect(childPrefix(`${APP_BASE}/docs`, "a.pdf")).toBe(
			`${APP_BASE}/docs/a.pdf`,
		);
	});

	// Proof the rule comes from the prefix, not from sniffing for "apps/".
	test("handles a folder the user named like the object-store base", () => {
		expect(normalizeStorageLocation(`${APP_BASE}/apps`, "")).toBe("apps");
		expect(normalizeStorageLocation(`${APP_BASE}/apps/a.pdf`, "apps")).toBe(
			"apps/a.pdf",
		);
		expect(normalizeStorageLocation("apps", "")).toBe("apps");
		expect(normalizeStorageLocation("apps/a.pdf", "apps")).toBe("apps/a.pdf");
		expect(
			normalizeStorageLocation("upload/upload/a.pdf", "upload/upload"),
		).toBe("upload/upload/a.pdf");
	});

	test("tolerates trailing and duplicated slashes on both sides", () => {
		expect(normalizeStorageLocation(`${APP_BASE}/docs/`, "")).toBe("docs");
		expect(normalizeStorageLocation("docs/", "")).toBe("docs");
		expect(normalizeStorageLocation("docs//a.pdf", "/docs/")).toBe(
			"docs/a.pdf",
		);
		expect(normalizeStorageLocation("", "docs")).toBe("docs");
	});
});

describe("storageItemName", () => {
	test("reads the last segment of either backend's shape", () => {
		expect(storageItemName(item(`${APP_BASE}/docs/a.pdf`))).toBe("a.pdf");
		expect(storageItemName(item("docs/a.pdf"))).toBe("a.pdf");
		expect(storageItemName(item("a.pdf"))).toBe("a.pdf");
	});

	test("ignores trailing slashes on folder keys", () => {
		expect(storageItemName(item(`${APP_BASE}/docs/`, true))).toBe("docs");
		expect(storageItemName(item("docs/", true))).toBe("docs");
		expect(storageItemName(item(""))).toBe("");
	});
});

describe("childPrefix and parentPrefix", () => {
	test("descend and ascend from the root", () => {
		expect(childPrefix("", "docs")).toBe("docs");
		expect(childPrefix("docs", "2026")).toBe("docs/2026");
		expect(childPrefix("docs/", "/2026/")).toBe("docs/2026");
		expect(childPrefix("docs", "")).toBe("docs");
		expect(parentPrefix("docs/2026/q1")).toBe("docs/2026");
		expect(parentPrefix("docs")).toBe("");
		expect(parentPrefix("")).toBe("");
	});

	test("drops traversal segments", () => {
		expect(childPrefix("docs", "../../etc")).toBe("docs/etc");
	});
});

describe("storagePrefixTrail", () => {
	test("walks the root down to the prefix", () => {
		expect(storagePrefixTrail("docs/2026/q1")).toEqual([
			"",
			"docs",
			"docs/2026",
			"docs/2026/q1",
		]);
		expect(storagePrefixTrail("")).toEqual([""]);
		expect(storagePrefixTrail("/docs/")).toEqual(["", "docs"]);
	});
});

describe("storageNodeId", () => {
	test("separates the two scopes at the same path", () => {
		expect(storageNodeId("app", "docs")).not.toBe(
			storageNodeId("user", "docs"),
		);
	});

	test("is stable across the two backend shapes once normalized", () => {
		const cloud = normalizeStorageLocation(`${APP_BASE}/docs/a.pdf`, "docs");
		const desktop = normalizeStorageLocation("docs/a.pdf", "docs");
		expect(storageNodeId("app", cloud)).toBe(storageNodeId("app", desktop));
	});

	test("round-trips through parseStorageNodeId", () => {
		expect(parseStorageNodeId(storageNodeId("user", "docs/2026"))).toEqual({
			scope: "user",
			prefix: "docs/2026",
		});
		expect(parseStorageNodeId(storageNodeId("app", ""))).toEqual({
			scope: "app",
			prefix: "",
		});
		expect(parseStorageNodeId(storageNodeId("app", "docs/a:b.pdf"))).toEqual({
			scope: "app",
			prefix: "docs/a:b.pdf",
		});
		expect(parseStorageNodeId("layer:abc")).toBeNull();
		expect(parseStorageNodeId("storage:other:docs")).toBeNull();
	});
});

describe("storageTreeEntry", () => {
	test("resolves a cloud listing into a path safe to list again", () => {
		const entry = storageTreeEntry(
			item(`${APP_BASE}/docs/2026`, true),
			"docs",
			"app",
		);
		expect(entry.path).toBe("docs/2026");
		expect(entry.name).toBe("2026");
		expect(entry.isFolder).toBe(true);
		expect(entry.nodeId).toBe(storageNodeId("app", "docs/2026"));
	});

	test("resolves a desktop listing to the identical path", () => {
		const cloud = storageTreeEntry(
			item(`${APP_BASE}/docs/a.pdf`),
			"docs",
			"app",
		);
		const desktop = storageTreeEntry(item("docs/a.pdf"), "docs", "app");
		expect(desktop.path).toBe(cloud.path);
		expect(desktop.nodeId).toBe(cloud.nodeId);
		expect(desktop.isFolder).toBe(false);
	});

	test("treats a missing is_dir as a file", () => {
		const entry = storageTreeEntry(
			{ location: "docs/a.pdf", last_modified: "", size: 12 },
			"docs",
			"user",
		);
		expect(entry.isFolder).toBe(false);
	});
});

describe("sortStorageEntries", () => {
	test("puts folders first, then names naturally", () => {
		const entries = [
			item("b.pdf"),
			item("folder-10", true),
			item("a.pdf"),
			item("folder-2", true),
		].map((entry) => storageTreeEntry(entry, "", "app"));

		expect(sortStorageEntries(entries).map((entry) => entry.name)).toEqual([
			"folder-2",
			"folder-10",
			"a.pdf",
			"b.pdf",
		]);
	});

	test("does not mutate its input", () => {
		const entries = [item("b.pdf"), item("a.pdf")].map((entry) =>
			storageTreeEntry(entry, "", "app"),
		);
		sortStorageEntries(entries);
		expect(entries.map((entry) => entry.name)).toEqual(["b.pdf", "a.pdf"]);
	});
});
