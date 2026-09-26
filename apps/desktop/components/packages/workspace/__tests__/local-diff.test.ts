import { describe, expect, test } from "vitest";
import { diffNodes, hasNodeChanges } from "../local-diff";

describe("diffNodes", () => {
	test("sorts nodes into added, removed and changed by name", () => {
		const diff = diffNodes(
			[
				{ name: "fetch", friendly_name: "Fetch", description: "Fetch a URL" },
				{ name: "parse", friendly_name: "Parse", description: "Parse JSON  " },
				{ name: "new_node", friendly_name: "New node", description: "" },
			],
			[
				{ name: "fetch", friendlyName: "Fetch", description: "Fetch an URL" },
				{ name: "parse", friendlyName: "Parse", description: "Parse  JSON" },
				{ name: "gone", friendlyName: "Gone", description: "Old node" },
			],
		);

		expect(diff.added).toEqual([{ name: "new_node", label: "New node" }]);
		expect(diff.removed).toEqual([{ name: "gone", label: "Gone" }]);
		expect(diff.changed).toEqual([{ name: "fetch", label: "Fetch" }]);
		expect(diff.unchanged).toBe(1);
		expect(hasNodeChanges(diff)).toBe(true);
	});

	test("falls back to the node name without a friendly name", () => {
		const diff = diffNodes([{ name: "a" }], [{ name: "b" }]);
		expect(diff.added).toEqual([{ name: "a", label: "a" }]);
		expect(diff.removed).toEqual([{ name: "b", label: "b" }]);
	});

	test("an identical build has no changes", () => {
		const nodes = [{ name: "a", description: "same" }];
		const diff = diffNodes(nodes, nodes);
		expect(hasNodeChanges(diff)).toBe(false);
		expect(diff.unchanged).toBe(1);
	});
});
