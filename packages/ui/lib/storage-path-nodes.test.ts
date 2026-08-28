import { describe, expect, test } from "bun:test";
import type { INode } from "./schema/flow/node";
import { buildStoragePathNodes } from "./storage-path-nodes";
import { convertJsonToUint8Array } from "./uint8";

function pin(id: string, name: string, pinType: "Input" | "Output") {
	return {
		id,
		name,
		friendly_name: name,
		pin_type: pinType,
		data_type: name === "child_name" ? "String" : "Struct",
		value_type: "Normal",
		depends_on: [],
		connected_to: [],
		index: 1,
		description: "",
	};
}

function node(name: string, pins: ReturnType<typeof pin>[]): INode {
	return {
		id: `${name}-template`,
		name,
		friendly_name: name,
		description: "",
		category: "Data/Files",
		coordinates: [0, 0, 0],
		pins: Object.fromEntries(pins.map((p) => [p.id, p])),
	} as unknown as INode;
}

const CATALOG = [
	node("path_from_upload_dir", [pin("u1", "path", "Output")]),
	node("path_from_user_dir", [
		pin("s1", "path", "Output"),
		pin("s2", "node_scope", "Input"),
	]),
	node("path_from_storage_dir", [pin("x1", "path", "Output")]),
	node("child", [
		pin("c1", "path", "Output"),
		pin("c2", "parent_path", "Input"),
		pin("c3", "child_name", "Input"),
	]),
] as INode[];

function named(result: { nodes: INode[] } | null, name: string) {
	return result?.nodes.find((entry) => entry.name === name);
}

function pinNamed(entry: INode | undefined, name: string) {
	return Object.values(entry?.pins ?? {}).find((p) => p.name === name);
}

describe("buildStoragePathNodes", () => {
	test("app storage is the upload dir, not the storage dir", () => {
		const built = buildStoragePathNodes({
			catalog: CATALOG,
			scope: "app",
			path: "docs/a.pdf",
		});
		expect(named(built, "path_from_upload_dir")).toBeDefined();
		expect(named(built, "path_from_storage_dir")).toBeUndefined();
	});

	test("user storage uses the user dir at app scope", () => {
		const built = buildStoragePathNodes({
			catalog: CATALOG,
			scope: "user",
			path: "a.txt",
		});
		const dir = named(built, "path_from_user_dir");
		expect(dir).toBeDefined();
		expect(pinNamed(dir, "node_scope")?.default_value).toEqual(
			convertJsonToUint8Array(false),
		);
	});

	test("wires the directory's path into the child's parent", () => {
		const built = buildStoragePathNodes({
			catalog: CATALOG,
			scope: "app",
			path: "a.txt",
		});
		const out = pinNamed(named(built, "path_from_upload_dir"), "path");
		const parent = pinNamed(named(built, "child"), "parent_path");
		expect(out?.connected_to).toEqual([parent?.id as string]);
		expect(parent?.depends_on).toEqual([out?.id as string]);
	});

	test("the whole relative path goes in one child — it splits on / at run time", () => {
		const built = buildStoragePathNodes({
			catalog: CATALOG,
			scope: "app",
			path: "deep/nested/report.csv",
		});
		expect(built?.nodes).toHaveLength(2);
		expect(pinNamed(named(built, "child"), "child_name")?.default_value).toEqual(
			convertJsonToUint8Array("deep/nested/report.csv"),
		);
	});

	test("short folder names survive intact", () => {
		const built = buildStoragePathNodes({
			catalog: CATALOG,
			scope: "app",
			path: "img",
		});
		expect(pinNamed(named(built, "child"), "child_name")?.default_value).toEqual(
			convertJsonToUint8Array("img"),
		);
	});

	test("every id is fresh, so dropping twice does not collide", () => {
		const first = buildStoragePathNodes({
			catalog: CATALOG,
			scope: "app",
			path: "a.txt",
		});
		const second = buildStoragePathNodes({
			catalog: CATALOG,
			scope: "app",
			path: "a.txt",
		});
		const ids = [...(first?.nodes ?? []), ...(second?.nodes ?? [])].map(
			(entry) => entry.id,
		);
		expect(new Set(ids).size).toBe(4);
		const pinIds = [...(first?.nodes ?? []), ...(second?.nodes ?? [])].flatMap(
			(entry) => Object.keys(entry.pins),
		);
		expect(new Set(pinIds).size).toBe(pinIds.length);
	});

	test("templates are not mutated, so the catalog stays reusable", () => {
		buildStoragePathNodes({ catalog: CATALOG, scope: "app", path: "a.txt" });
		const template = CATALOG.find((n) => n.name === "child");
		expect(pinNamed(template, "child_name")?.default_value).toBeUndefined();
		expect(pinNamed(template, "parent_path")?.depends_on).toEqual([]);
	});

	test("the two nodes do not land on top of each other", () => {
		const built = buildStoragePathNodes({
			catalog: CATALOG,
			scope: "app",
			path: "a.txt",
			position: { x: 100, y: 50 },
		});
		const dir = named(built, "path_from_upload_dir");
		const child = named(built, "child");
		expect(dir?.coordinates?.[0]).toBe(100);
		expect(child?.coordinates?.[0]).toBeGreaterThan(
			dir?.coordinates?.[0] as number,
		);
		expect(child?.coordinates?.[1]).toBe(50);
	});

	test("returns null rather than half a fragment when the catalog is short", () => {
		expect(
			buildStoragePathNodes({ catalog: [], scope: "app", path: "a.txt" }),
		).toBeNull();
		expect(
			buildStoragePathNodes({
				catalog: undefined,
				scope: "app",
				path: "a.txt",
			}),
		).toBeNull();
		expect(
			buildStoragePathNodes({
				catalog: CATALOG.filter((n) => n.name !== "child"),
				scope: "app",
				path: "a.txt",
			}),
		).toBeNull();
	});
});
