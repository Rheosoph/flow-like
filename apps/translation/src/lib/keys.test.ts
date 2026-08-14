import { describe, expect, test } from "bun:test";
import {
	buildRows,
	coverageOf,
	displayKey,
	flatten,
	missingPlaceholders,
	placeholders,
	statusOf,
	unflatten,
} from "./keys";

describe("translation keys", () => {
	test("normalizes runtime tokens and preserves duplicates", () => {
		expect(
			placeholders("Hi {{ name }} $t( common.next ) <0>x</0><1/>"),
		).toEqual(["$t(common.next)", "</0>", "<0>", "<1/>", "{{name}}"]);
		expect(missingPlaceholders("{{name}} {{name}}", "{{name}}")).toEqual([
			"{{name}}",
		]);
	});

	test("marks dropped React Trans tags as broken", () => {
		expect(statusOf("Open <0>settings</0>", "Einstellungen öffnen")).toBe(
			"broken",
		);
		expect(statusOf("Line<0/>", "Zeile<0/>")).toBe("translated");
		expect(statusOf("<0>One</0> and <0>two</0>", "<0>Eins</0>")).toBe("broken");
	});

	test("coverage excludes orphans from source work but still flags them", () => {
		const rows = buildRows(
			{
				en: { common: { copied: "Flow-Like", missing: "Missing" } },
				de: {
					common: { copied: "Flow-Like", missing: "", old: "Alt" },
				},
			},
			["common"],
			"en",
			"de",
		);
		expect(coverageOf(rows)).toEqual({
			total: 2,
			complete: 1,
			translated: 0,
			missing: 1,
			problems: 2,
			percent: 50,
		});
	});

	test("unflatten keeps dangerous-looking keys as own data", () => {
		const dangerous = JSON.parse(
			'{"__proto__":{"translated":"yes"}}',
		) as Record<string, unknown>;
		const flat = flatten(dangerous);
		const result = unflatten(flat);
		expect(Object.hasOwn(result, "__proto__")).toBe(true);
		expect((result.__proto__ as Record<string, string>).translated).toBe("yes");
		expect(({} as Record<string, unknown>).translated).toBeUndefined();
	});

	test("round-trips literal dots without confusing them with nesting", () => {
		const tree = { "literal.key": "Literal", nested: { child: "Child" } };
		const flat = flatten(tree);
		expect(Object.keys(flat).map(displayKey)).toEqual([
			"literal.key",
			"nested.child",
		]);
		expect(unflatten(flat)).toEqual(tree);
	});
});
