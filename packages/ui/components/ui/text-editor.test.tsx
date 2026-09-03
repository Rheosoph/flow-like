import { describe, expect, test } from "bun:test";
import { PlateStatic, createSlateEditor } from "platejs";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { BaseEditorKit } from "../editor/editor-base-kit";
import { indexEditorPaths } from "./lazy-plate-static";
import {
	RICH_REMARK_PLUGINS,
	TextEditor,
	resolveStaticValue,
} from "./text-editor";

describe("TextEditor", () => {
	test("renders complete Plate JSON documents larger than 50,000 characters", () => {
		const content = `plate_json::${JSON.stringify([
			{
				type: "p",
				children: [{ text: `start-${"x".repeat(50_100)}-end-sentinel` }],
			},
		])}`;

		const html = renderToStaticMarkup(
			createElement(TextEditor, { initialContent: content, isMarkdown: true }),
		);

		expect(html).toContain("start-");
		expect(html).toContain("end-sentinel");
		expect(html).not.toContain("plate_json::");
		expect(html).not.toContain("content truncated for performance");
	});
});

describe("TextEditor markdown tables", () => {
	const markdown = [
		"| Category | Nodes | What's inside |",
		"| --- | --- | --- |",
		"| Digital Twin | 6 | Direct DTR access: create/get/delete shells, add submodels |",
		"| Discovery | 4 | Discovery Finder, BPN Discovery, EDC Discovery and bridges |",
	].join("\n");

	const html = renderToStaticMarkup(
		createElement(TextEditor, { initialContent: markdown, isMarkdown: true }),
	);

	test("renders one visible table and keeps the Slate tracking table hidden", () => {
		expect(html.match(/<table/g)?.length).toBe(2);
		expect(html.match(/<table style="display:none"/g)?.length).toBe(1);
	});

	test("does not force descendant tables to display:block", () => {
		expect(html).not.toContain("_table]:block");
	});

	test("offers a way to reveal truncated cell text", () => {
		expect(html).toContain("Click to expand");
		expect(html).toContain(">Wrap<");
		expect(html).toContain(
			"Direct DTR access: create/get/delete shells, add submodels",
		);
	});
});

/** React useId and dnd-kit stamp render-position counters, not content. */
const stripGeneratedIds = (html: string) =>
	html
		.replace(/DndDescribedBy-\d+/g, "DndDescribedBy-N")
		.replace(/radix-[\w-]+/g, "radix-N")
		.replace(/«[^»]*»/g, "«N»")
		.replace(/_r_[0-9a-z]+_/g, "_rN_");

describe("static parse cache", () => {
	const markdown = [
		"# Cached",
		"- one\n  - nested\n- two",
		"| A | B |\n| --- | --- |\n| 1 | 2 |",
		"```ts\nconst y = 2;\n```",
		"> quoted",
	].join("\n\n");

	const resolve = (content: string, isMarkdown = true, kit = BaseEditorKit) =>
		resolveStaticValue(kit, content, isMarkdown, RICH_REMARK_PLUGINS);

	test("re-mounts of the same content share one parsed value", () => {
		const first = resolve(markdown);
		expect(resolve(markdown)).toBe(first);
	});

	test("kit identity and markdown mode are part of the key", () => {
		const rich = resolve(markdown);
		expect(resolve(markdown, false)).not.toBe(rich);
		expect(resolve(markdown, true, [...BaseEditorKit])).not.toBe(rich);
	});

	test("the cache is bounded", () => {
		const original = resolve("evict 0");
		for (let i = 1; i <= 40; i++) resolve(`evict ${i}`);
		expect(resolve("evict 0")).not.toBe(original);
	});

	// The caches hand one value to every editor built from it, which is only
	// sound while neither editor creation nor static rendering writes to a
	// node. Fails the moment a plugin's initial normalisation starts mutating —
	// then the caches must clone on the way out.
	test("editors built on a shared value never write to it", () => {
		const value = resolve(`${markdown}\n\nmutation guard`);
		const snapshot = JSON.stringify(value);
		const blocks = [...value];

		const render = () => {
			const editor = createSlateEditor({
				plugins: BaseEditorKit,
				value,
				nodeId: false,
			});
			indexEditorPaths(editor);
			const html = renderToStaticMarkup(createElement(PlateStatic, { editor }));
			return { editor, html };
		};
		const first = render();
		const second = render();

		expect(first.editor.children).toBe(value);
		expect(second.editor.children).toBe(value);
		blocks.forEach((block, index) => expect(value[index]).toBe(block));
		expect(JSON.stringify(value)).toBe(snapshot);
		expect(Object.isFrozen(value)).toBe(false);
		expect(blocks.some((block) => Object.isFrozen(block))).toBe(false);
		expect(stripGeneratedIds(first.html)).toBe(stripGeneratedIds(second.html));
	});
});
