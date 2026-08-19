import { describe, expect, test } from "bun:test";
import { normalizeTargetTree } from "./locales-plugin";

describe("locale writer normalization", () => {
	test("keeps the exact source keyset, including empty translations", () => {
		expect(
			normalizeTargetTree(
				{ greeting: "Hello", nested: { saved: "Saved" } },
				{ greeting: "Hallo", orphan: "old", nested: { extra: "old" } },
			),
		).toEqual({ greeting: "Hallo", nested: { saved: "" } });
	});

	test("preserves nested source keys with literal dotted path segments", () => {
		const source = {
			"literal.key": "Literal",
			nested: { child: "Child" },
		};
		const target = {
			"literal.key": "Wörtlich",
			nested: { child: "Kind" },
		};
		expect(normalizeTargetTree(source, target)).toEqual(target);
	});
});
