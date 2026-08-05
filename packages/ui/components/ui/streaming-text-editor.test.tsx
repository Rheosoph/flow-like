import { describe, expect, test } from "bun:test";
import { createSlateEditor } from "platejs";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { BaseEditorKit } from "../editor/editor-base-kit";
import {
	EMPTY_STREAMING_STATE,
	type StreamingParseState,
	parseStreamingMarkdown,
	splitStreamingBlocks,
} from "./streaming-markdown-blocks";
import { StreamingTextEditor } from "./streaming-text-editor";
import { RICH_REMARK_PLUGINS, TextEditor, safeDeserialize } from "./text-editor";

const worker = createSlateEditor({ plugins: BaseEditorKit, nodeId: false });

const wholeDocumentParse = (markdown: string) =>
	safeDeserialize(worker, markdown, true, RICH_REMARK_PLUGINS);

/** Streams `markdown` one character at a time, carrying parse state forward. */
function streamChars(markdown: string) {
	let state: StreamingParseState = EMPTY_STREAMING_STATE;
	const steps: Array<{ prefix: string; state: StreamingParseState }> = [];
	for (let i = 1; i <= markdown.length; i++) {
		const prefix = markdown.slice(0, i);
		state = parseStreamingMarkdown(worker, prefix, state);
		steps.push({ prefix, state });
	}
	return steps;
}

/**
 * Documents that exercise constructs whose meaning spans block boundaries —
 * the cases where naive per-block parsing diverges from a whole-document parse.
 */
const CORPUS: ReadonlyArray<readonly [string, string]> = [
	[
		"mixed report",
		"# Report\n\nProse with **bold**, `code` and a [link](https://x.dev).\n\n## Detail\n\n- alpha\n- beta\n\n1. first\n2. second\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n```ts\nconst x = 1;\n```\n\n> quoted\n\nDone.",
	],
	["loose ordered list", "1. one\n\n2. two\n\n3. three\n\nAfter."],
	["all-ones ordered list", "1. a\n\n1. b\n\n1. c"],
	["odd-start ordered list", "5. five\n\n9. nine\n\n2. two"],
	["loose bullet list", "- a\n\n- b\n\n- c"],
	["list with continuation", "1. item\n\n   continuation paragraph\n\n2. next"],
	["nested list", "- a\n\n  - b\n\n  - c\n\n- d"],
	["list containing a fence", "- item\n\n  ```ts\n  const a = 1;\n  ```\n\n- next"],
	["directive block", ":::info\nSome info\n\nWith a blank line\n:::\n\nAfter."],
	["reference link", "See [the docs][ref] for more.\n\n[ref]: https://x.dev"],
	["html block", "<div>\nhi\n</div>\n\nAfter."],
	["table then prose", "| A | B |\n| --- | --- |\n| 1 | 2 |\n\nAfter the table."],
	[
		"dollar amounts and block math",
		"It costs $5 today and $10 tomorrow.\n\nBlock: $$a^2 + b^2$$",
	],
	["setext headings", "Title\n===\n\nBody text.\n\nSub\n---\n\nMore."],
	["thematic break", "Above.\n\n---\n\nBelow."],
	["fence with blank lines", "Intro.\n\n```js\nconst a = 1;\n\nconst b = 2;\n```\n\nEnd."],
];

describe("streaming parse equals whole-document parse at every prefix", () => {
	for (const [name, markdown] of CORPUS) {
		test(name, () => {
			for (const { prefix, state } of streamChars(markdown)) {
				expect({ prefix, value: state.blocks }).toEqual({
					prefix,
					value: wholeDocumentParse(prefix),
				});
			}
		});
	}
});

describe("streaming tail integrity", () => {
	test("an unterminated fence stays one tail block", () => {
		expect(splitStreamingBlocks("Intro.\n\n```js\nconst a = 1;\n\nco")).toEqual([
			"Intro.",
			"```js\nconst a = 1;\n\nco",
		]);
	});

	test("a half-written table row is not frozen into the cache", () => {
		const partial = "Intro.\n\n| A | B |";
		const blocks = splitStreamingBlocks(partial);
		expect(blocks[blocks.length - 1]).toContain("| A | B |");
		// The row only becomes a table once the separator arrives; it must not have
		// been cached as a paragraph before then.
		const grown = parseStreamingMarkdown(
			worker,
			`${partial}\n| --- | --- |\n| 1 | 2 |`,
			parseStreamingMarkdown(worker, partial, EMPTY_STREAMING_STATE),
		);
		expect(grown.blocks).toEqual(
			wholeDocumentParse(`${partial}\n| --- | --- |\n| 1 | 2 |`),
		);
	});
});

describe("streaming cache", () => {
	const unit =
		"## Section\n\nSome prose with **bold** text and a bit more to say here.\n\n- alpha\n- beta\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n```ts\nconst x = 1;\n```\n\n> quoted\n";
	const document = Array.from({ length: 16 }, () => unit).join("\n");

	function streamChunks(size: number) {
		let state: StreamingParseState = EMPTY_STREAMING_STATE;
		const states: StreamingParseState[] = [];
		for (let i = size; i <= document.length + size; i += size) {
			state = parseStreamingMarkdown(
				worker,
				document.slice(0, Math.min(i, document.length)),
				state,
			);
			states.push(state);
		}
		return states;
	}

	test("blocks before firstChangedBlock keep object identity", () => {
		const states = streamChunks(24);
		let compared = 0;

		for (let i = 1; i < states.length; i++) {
			const previous = states[i - 1];
			const current = states[i];
			expect(current.firstChangedBlock).toBeLessThanOrEqual(
				current.blocks.length,
			);
			for (let b = 0; b < current.firstChangedBlock; b++) {
				expect(current.blocks[b]).toBe(previous.blocks[b]);
				compared++;
			}
		}

		expect(compared).toBeGreaterThan(1000);
	});

	test("the final state matches a whole-document parse", () => {
		const states = streamChunks(24);
		expect(states[states.length - 1].blocks).toEqual(
			wholeDocumentParse(document),
		);
	});

	test("incremental parsing beats re-parsing the whole document", () => {
		const prefixes: string[] = [];
		for (let i = 24; i <= document.length; i += 24)
			prefixes.push(document.slice(0, i));

		let state: StreamingParseState = EMPTY_STREAMING_STATE;
		const incrementalStart = performance.now();
		for (const prefix of prefixes)
			state = parseStreamingMarkdown(worker, prefix, state);
		const incremental = performance.now() - incrementalStart;

		const wholeStart = performance.now();
		for (const prefix of prefixes) wholeDocumentParse(prefix);
		const whole = performance.now() - wholeStart;

		// Measured ~12x on the dev machine; assert a ratio, not absolute ms, so the
		// guard survives slower CI hardware.
		expect(incremental * 4).toBeLessThan(whole);
	});
});

describe("non-prefix content updates", () => {
	// buildInlineSegments re-slices the live segment when a plan step anchors
	// mid-reply, so content can shrink or shift its start between frames.
	const original = "# A\n\nlong body paragraph\n\n## B\n\ntail";

	for (const [name, replacement] of [
		["a shorter unrelated string", "totally different"],
		["a string sharing only the first block", "# A\n\nsomething else"],
		["the empty string", ""],
	] as const) {
		test(`recovers from ${name}`, () => {
			const first = parseStreamingMarkdown(
				worker,
				original,
				EMPTY_STREAMING_STATE,
			);
			const second = parseStreamingMarkdown(worker, replacement, first);

			expect(second.blocks).toEqual(
				replacement
					? wholeDocumentParse(replacement)
					: [{ type: "p", children: [{ text: "" }] }],
			);
		});
	}
});

/** React useId, dnd-kit and Radix stamp render-position counters, not content. */
const stripGeneratedIds = (html: string) =>
	html
		.replace(/DndDescribedBy-\d+/g, "DndDescribedBy-N")
		.replace(/radix-[\w-]+/g, "radix-N")
		.replace(/«[^»]*»/g, "«N»")
		.replace(/:r[0-9a-z]+:/g, ":rN:");

describe("streaming render matches the settled render", () => {
	const CONSTRUCTS: ReadonlyArray<readonly [string, string]> = [
		["heading", "# Heading one"],
		["marks", "Text with **bold**, _italic_, ~~strike~~ and `code`."],
		["link", "A [link](https://example.com) inline."],
		["image", "![alt text](https://example.com/x.png)"],
		["bullet list", "- alpha\n- beta\n- gamma"],
		["ordered list starting at 7", "7. seven\n8. eight\n9. nine"],
		["table", "| A | B |\n| --- | --- |\n| 1 | [two](https://x.dev) |"],
		["code fence", "```ts\nconst x: number = 1;\n```"],
		["blockquote", "> quoted line\n> second line"],
		["thematic break", "Above.\n\n---\n\nBelow."],
		// The remark math option used to differ between the two paths, so `$5 … $10`
		// became an inline equation mid-stream and repaired itself on settle.
		// KaTeX itself cannot be server-rendered in this harness (DOMPurify needs a
		// DOM) on either path, so `$$…$$` is covered by the parse tests instead.
		["dollar amounts", "It costs $5 today and $10 tomorrow."],
		["directive", ":::info\nSome info\n:::"],
	];

	for (const [name, markdown] of CONSTRUCTS) {
		test(name, () => {
			const streaming = stripGeneratedIds(
				renderToStaticMarkup(
					createElement(StreamingTextEditor, { content: markdown }),
				),
			);
			const settled = stripGeneratedIds(
				renderToStaticMarkup(
					createElement(TextEditor, {
						initialContent: markdown,
						isMarkdown: true,
						editable: false,
					}),
				),
			);

			expect(streaming).toBe(settled);
		});
	}

	test("void nodes keep the attribute chat styling depends on", () => {
		// global.css de-serifs embedded widgets through [data-fl-chat-prose]
		// [data-slate-void], so the renderer swap must keep emitting it.
		const html = renderToStaticMarkup(
			createElement(StreamingTextEditor, {
				content: "![alt](https://example.com/x.png)",
			}),
		);
		expect(html).toContain("data-slate-void");
		expect(html).toContain("slate-editor");
	});

	test("empty content renders a single empty paragraph without throwing", () => {
		const html = renderToStaticMarkup(
			createElement(StreamingTextEditor, { content: "" }),
		);
		expect(html).toContain("data-slate-editor");
		expect((html.match(/data-slate-node="element"/g) ?? []).length).toBe(1);
	});
});
