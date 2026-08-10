import { describe, expect, test } from "bun:test";
import {
	buildCategoryTree,
	collectFolderPaths,
	compareByNameThenId,
	folderDroppableId,
	normalizeCategory,
	parseFolderDroppableId,
} from "./category-tree";

const item = (id: string, name: string, category?: string | null) => ({
	id,
	name,
	category,
});

describe("buildCategoryTree", () => {
	test("files uncategorized items at the root", () => {
		const tree = buildCategoryTree([
			item("1", "a"),
			item("2", "b", null),
			item("3", "c", "   "),
		]);

		expect(tree.items.map((i) => i.id)).toEqual(["1", "2", "3"]);
		expect(Object.keys(tree.children)).toEqual([]);
	});

	test("nests along '/' and trims segments", () => {
		const tree = buildCategoryTree([
			item("1", "a", "Utils/ Math "),
			item("2", "b", "Utils"),
			item("3", "c", "/Utils//Math/"),
		]);

		expect(tree.children.Utils.items.map((i) => i.id)).toEqual(["2"]);
		expect(tree.children.Utils.children.Math.items.map((i) => i.id)).toEqual([
			"1",
			"3",
		]);
		expect(tree.children.Utils.children.Math.path).toBe("Utils/Math");
	});
});

describe("compareByNameThenId", () => {
	test("orders by name and breaks ties on id", () => {
		const sorted = [
			item("z", "beta"),
			item("b", "alpha"),
			item("a", "alpha"),
		].sort(compareByNameThenId);

		expect(sorted.map((i) => i.id)).toEqual(["a", "b", "z"]);
	});

	test("is independent of input order", () => {
		const items = [
			item("3", "gamma"),
			item("1", "alpha"),
			item("2", "alpha"),
			item("4", "beta"),
		];
		const forward = [...items].sort(compareByNameThenId).map((i) => i.id);
		const backward = [...items]
			.reverse()
			.sort(compareByNameThenId)
			.map((i) => i.id);

		expect(forward).toEqual(backward);
	});
});

describe("collectFolderPaths", () => {
	test("returns every folder depth-first and sorted", () => {
		const tree = buildCategoryTree([
			item("1", "a", "Utils/Math"),
			item("2", "b", "Api"),
			item("3", "c", "Utils/Strings"),
		]);

		expect(collectFolderPaths(tree)).toEqual([
			"Api",
			"Utils",
			"Utils/Math",
			"Utils/Strings",
		]);
	});
});

describe("folder droppable ids", () => {
	test("round-trips kind and path", () => {
		expect(parseFolderDroppableId(folderDroppableId("functions", ""))).toEqual({
			kind: "functions",
			path: "",
		});
		expect(
			parseFolderDroppableId(folderDroppableId("variables", "Utils/Math")),
		).toEqual({ kind: "variables", path: "Utils/Math" });
	});

	test("ignores foreign droppable ids", () => {
		expect(parseFolderDroppableId("flow")).toBeNull();
		expect(parseFolderDroppableId("folder:functions")).toBeNull();
	});
});

describe("normalizeCategory", () => {
	test("maps empty input to the top level", () => {
		expect(normalizeCategory("")).toBeUndefined();
		expect(normalizeCategory(null)).toBeUndefined();
		expect(normalizeCategory(" / ")).toBeUndefined();
		expect(normalizeCategory(" Utils / Math ")).toBe("Utils/Math");
	});
});
