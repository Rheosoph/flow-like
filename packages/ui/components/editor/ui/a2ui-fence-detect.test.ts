import { describe, expect, test } from "bun:test";
import manifest from "../../../../core/a2ui-schema/src/component-manifest.json";
import {
	CHAT_A2UI_COMPONENT_TYPES,
	isA2UIFence,
	looksLikeA2UIComponents,
} from "./a2ui-fence-detect";

const COMPONENT_LINES = [
	'{"id":"root","type":"column","gap":"14px","children":["headline","grid"]}',
	'{"id":"headline","type":"text","content":"Samstag, 24. Oktober","variant":"heading"}',
].join("\n");

describe("a2ui fence detection", () => {
	test("the inline type list matches the manifest's chat components", () => {
		expect([...CHAT_A2UI_COMPONENT_TYPES].sort()).toEqual(
			[...manifest.chat.components].sort(),
		);
	});

	test("an a2ui tag always renders, in any case and outside chats", () => {
		expect(isA2UIFence("a2ui", "", false)).toBe(true);
		expect(isA2UIFence(" A2UI ", "anything", false)).toBe(true);
	});

	test("json and untagged fences of component lines render in chats only", () => {
		for (const lang of ["json", "jsonl", "", undefined]) {
			expect(isA2UIFence(lang, COMPONENT_LINES, true)).toBe(true);
			expect(isA2UIFence(lang, COMPONENT_LINES, false)).toBe(false);
		}
		expect(isA2UIFence("typescript", COMPONENT_LINES, true)).toBe(false);
	});

	test("recognises a component while its first line is still streaming", () => {
		expect(looksLikeA2UIComponents('{"id":"root","type":"grid","col')).toBe(
			true,
		);
		expect(looksLikeA2UIComponents('{"id":"root","ty')).toBe(false);
	});

	test("accepts wrapped and nested authoring forms", () => {
		expect(
			looksLikeA2UIComponents('[{"id":"root","type":"card","title":"A"}]'),
		).toBe(true);
		expect(
			looksLikeA2UIComponents(
				'{\n  "id": "root",\n  "component": {\n    "type": "column"\n  }\n}',
			),
		).toBe(true);
		expect(
			looksLikeA2UIComponents(
				'{"components":[{"id":"root","type":"column","children":[]}]}',
			),
		).toBe(true);
	});

	test("leaves ordinary JSON as code", () => {
		for (const json of [
			'{"type":"object","properties":{"id":{"type":"string"}}}',
			'{"id":"evt_1","type":"invoice.paid"}',
			'{"id":42,"type":"text"}',
			'{"type":"text","text":"hello"}',
			"[1, 2, 3]",
			"plain text",
		]) {
			expect(looksLikeA2UIComponents(json)).toBe(false);
		}
	});
});
