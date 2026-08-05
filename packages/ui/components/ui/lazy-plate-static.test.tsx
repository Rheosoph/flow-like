import { describe, expect, test } from "bun:test";
import { PlateStatic, createSlateEditor } from "platejs";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { BaseEditorKit } from "../editor/editor-base-kit";
import {
	WINDOWING_BLOCK_THRESHOLD,
	indexEditorPaths,
} from "./lazy-plate-static";
import { TextEditor, safeDeserialize } from "./text-editor";

const remarkPlugins = [remarkGfm, remarkBreaks];

/** One of every block type whose renderer is path- or decoration-sensitive. */
const RICH_BLOCK = [
	"# Title",
	"Text with **bold**, _italic_, `code` and a [link](https://example.com).",
	"- alpha\n- beta",
	"1. first\n2. second",
	"| Col A | Col B |\n| --- | --- |\n| 1 | two |",
	"```ts\nconst x: number = 1;\nexport default x;\n```",
	"> a blockquote",
].join("\n\n");

function buildEditor(markdown: string, withPathIndex: boolean) {
	const probe = createSlateEditor({ plugins: BaseEditorKit });
	const value = safeDeserialize(probe, markdown, true, remarkPlugins);
	const editor = createSlateEditor({
		plugins: BaseEditorKit,
		value,
		nodeId: false,
	});
	if (withPathIndex) indexEditorPaths(editor);
	return editor;
}

/** dnd-kit and React's useId stamp render-position counters that are not content. */
const stripGeneratedIds = (html: string) =>
	html
		.replace(/DndDescribedBy-\d+/g, "DndDescribedBy-N")
		.replace(/radix-[\w-]+/g, "radix-N")
		.replace(/«[^»]*»/g, "«N»");

function renderMarkdown(markdown: string) {
	return renderToStaticMarkup(
		createElement(TextEditor, { initialContent: markdown, isMarkdown: true }),
	);
}

describe("indexEditorPaths", () => {
	test("renders byte-identical output to Plate's own findPath", () => {
		const stock = stripGeneratedIds(
			renderToStaticMarkup(
				createElement(PlateStatic, { editor: buildEditor(RICH_BLOCK, false) }),
			),
		);
		const indexed = stripGeneratedIds(
			renderToStaticMarkup(
				createElement(PlateStatic, { editor: buildEditor(RICH_BLOCK, true) }),
			),
		);

		expect(indexed).toBe(stock);
	});

	test("keeps decoration-driven syntax highlighting intact", () => {
		const indexed = renderToStaticMarkup(
			createElement(PlateStatic, { editor: buildEditor(RICH_BLOCK, true) }),
		);
		const stock = renderToStaticMarkup(
			createElement(PlateStatic, { editor: buildEditor(RICH_BLOCK, false) }),
		);

		const tokens = (html: string) => (html.match(/hljs-/g) ?? []).length;
		expect(tokens(indexed)).toBeGreaterThan(0);
		expect(tokens(indexed)).toBe(tokens(stock));
	});

	test("resolves a real node's path and returns undefined for a foreign node", () => {
		const editor = buildEditor(RICH_BLOCK, true);
		const [firstBlock] = editor.children;

		expect(editor.api.findPath(firstBlock)).toEqual([0]);
		expect(
			editor.api.findPath({ text: "not in this document" }),
		).toBeUndefined();
	});
});

describe("static markdown windowing", () => {
	const shortDoc = RICH_BLOCK;
	const longDoc = Array.from({ length: 60 }, (_, i) =>
		RICH_BLOCK.replace("# Title", `# Section ${i}`),
	).join("\n\n");

	test("documents at or below the threshold are not windowed", () => {
		const blocks = buildEditor(shortDoc, false).children.length;
		expect(blocks).toBeLessThanOrEqual(WINDOWING_BLOCK_THRESHOLD);

		const html = renderMarkdown(shortDoc);
		expect(html).not.toContain("data-slate-placeholder");
		expect(html).toContain("a blockquote");
	});

	test("long documents render the head eagerly and defer the tail", () => {
		const blocks = buildEditor(longDoc, false).children.length;
		expect(blocks).toBeGreaterThan(WINDOWING_BLOCK_THRESHOLD);

		const html = renderMarkdown(longDoc);

		expect(html).toContain("Section 0");
		expect(html).toContain("data-slate-placeholder");
		// The tail is deferred until it scrolls into view.
		expect(html).not.toContain("Section 59");
	});

	test("deferred chunks reserve height so the scrollbar stays sane", () => {
		const html = renderMarkdown(longDoc);
		const placeholders = [
			...html.matchAll(/data-slate-placeholder="true" style="height:(\d+)px"/g),
		];

		expect(placeholders.length).toBeGreaterThan(0);
		for (const [, height] of placeholders) {
			expect(Number(height)).toBeGreaterThan(0);
		}
	});

	test("a long document renders in a fraction of the unwindowed cost", () => {
		// Before windowing this document took minutes: every block rendered, and
		// each node's path was resolved with a full-document scan.
		const started = performance.now();
		renderMarkdown(longDoc);
		const elapsed = performance.now() - started;

		expect(elapsed).toBeLessThan(5_000);
	});
});
