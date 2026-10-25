import { describe, expect, test } from "bun:test";
import type { IStorageItem } from "./schema/storage/storage-item";
import {
	childPrefix,
	decodeStorageSegment,
	encodeStorageSegment,
	normalizeStorageLocation,
	parentPrefix,
	parseStorageNodeId,
	sortStorageEntries,
	storageDisplayName,
	storageDisplayPath,
	storageItemName,
	storageNodeId,
	storagePrefixTrail,
	storageTreeEntry,
} from "./storage-tree";

const APP_BASE = "apps/app-1/upload";
const USER_BASE = "users/sub-1/apps/app-1";
const RAW_NAME = "Übersicht (2)#1.pdf";
const ENCODED_NAME = "%C3%9Cbersicht (2)%231.pdf";

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

describe("decodeStorageSegment", () => {
	test("gives the raw name back for a listed key", () => {
		expect(decodeStorageSegment(ENCODED_NAME)).toBe(RAW_NAME);
		expect(decodeStorageSegment("100%25.txt")).toBe("100%.txt");
	});

	test("passes a raw name through unchanged", () => {
		expect(decodeStorageSegment(RAW_NAME)).toBe(RAW_NAME);
		expect(decodeStorageSegment("a.pdf")).toBe("a.pdf");
	});

	test("keeps a malformed escape as it is", () => {
		expect(decodeStorageSegment("%zz.txt")).toBe("%zz.txt");
		expect(decodeStorageSegment("trailing%")).toBe("trailing%");
	});
});

describe("encodeStorageSegment", () => {
	test("produces the listed key for a display name", () => {
		expect(encodeStorageSegment(RAW_NAME)).toBe(ENCODED_NAME);
		expect(encodeStorageSegment("Übersicht")).toBe("%C3%9Cbersicht");
		expect(encodeStorageSegment("100%.txt")).toBe("100%25.txt");
		expect(encodeStorageSegment("a/b|c*?.txt")).toBe("a%2Fb%7Cc%2A%3F.txt");
	});

	test("encodes dot segments whole and keeps single dots in names", () => {
		expect(encodeStorageSegment(".")).toBe("%2E");
		expect(encodeStorageSegment("..")).toBe("%2E%2E");
		expect(encodeStorageSegment("a.pdf")).toBe("a.pdf");
	});

	test("round-trips through decodeStorageSegment", () => {
		for (const name of [RAW_NAME, 'Ünïcode ✓ [1] {x} ~y`z"', "tab\there"]) {
			expect(decodeStorageSegment(encodeStorageSegment(name))).toBe(name);
		}
	});
});

describe("storageDisplayName and storageDisplayPath", () => {
	test("show the same name for a raw and a listed location", () => {
		expect(storageDisplayName(`${APP_BASE}/docs/${ENCODED_NAME}`)).toBe(
			RAW_NAME,
		);
		expect(storageDisplayName(`docs/${RAW_NAME}`)).toBe(RAW_NAME);
		expect(storageDisplayName(`docs/${ENCODED_NAME}`)).toBe(
			storageDisplayName(`docs/${RAW_NAME}`),
		);
	});

	test("decode every segment while keeping the separators", () => {
		expect(storageDisplayPath(`%C3%9Cbersicht/2026%231/${ENCODED_NAME}`)).toBe(
			`Übersicht/2026#1/${RAW_NAME}`,
		);
		expect(storageDisplayPath("docs/a.pdf")).toBe("docs/a.pdf");
		expect(storageDisplayPath("")).toBe("");
	});

	test("do not fold an encoded slash into a separator", () => {
		expect(storageDisplayPath("docs/a%2Fb.pdf")).toBe("docs/a/b.pdf");
		expect(storageDisplayPath("docs/a%2Fb.pdf").split("/")).toHaveLength(3);
		expect(storageDisplayName("docs/a%2Fb.pdf")).toBe("a/b.pdf");
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

	test("keeps the listed key for addressing and decodes only the name", () => {
		const cloud = storageTreeEntry(
			item(`${APP_BASE}/docs/${ENCODED_NAME}`),
			"docs",
			"app",
		);
		const desktop = storageTreeEntry(
			item(`docs/${ENCODED_NAME}`),
			"docs",
			"app",
		);
		expect(cloud.path).toBe(`docs/${ENCODED_NAME}`);
		expect(desktop.path).toBe(cloud.path);
		expect(desktop.nodeId).toBe(cloud.nodeId);
		expect(cloud.name).toBe(RAW_NAME);
		expect(storageItemName(cloud.item)).toBe(RAW_NAME);
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
