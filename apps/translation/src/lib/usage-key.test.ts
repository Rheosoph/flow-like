import { expect, test } from "bun:test";
import { flatten } from "./keys";
import { usageNeedles } from "./usage-key";

test("non-default namespace usage search covers bound and prefixed calls", () => {
	const [key] = Object.keys(flatten({ nested: { title: "Title" } }));
	expect(usageNeedles(key, "flow", "common")).toEqual([
		"flow:nested.title",
		"nested.title",
	]);
});
