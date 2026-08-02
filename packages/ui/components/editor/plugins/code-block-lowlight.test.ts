import { describe, expect, test } from "bun:test";
import { createEditorLowlight } from "./code-block-lowlight";
import { DIRECTIVE_TYPES } from "./remark-directives";

const CUSTOM_FENCES = [
	"nivo",
	"plotly",
	"embed",
	"map",
	...DIRECTIVE_TYPES.map((type) => `directive-${type}`),
];

describe("editor lowlight", () => {
	test("highlights every custom fence language without throwing", () => {
		const lowlight = createEditorLowlight();

		for (const language of CUSTOM_FENCES) {
			expect(lowlight.registered(language)).toBe(true);
			expect(() => lowlight.highlight(language, "type: bar")).not.toThrow();
		}
	});

	test("still highlights the languages the editor ships with", () => {
		const lowlight = createEditorLowlight();

		expect(lowlight.registered("typescript")).toBe(true);
		expect(lowlight.highlight("json", '{"a":1}').children.length).toBeGreaterThan(
			0,
		);
	});
});
