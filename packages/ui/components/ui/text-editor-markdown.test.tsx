import { describe, expect, test } from "bun:test";
import { type Value, createSlateEditor } from "platejs";
import { PlateStatic } from "platejs/static";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
	CODE_MARKUP_FIXTURES,
	MARKDOWN_FIXTURES,
	STORED_FIXTURES,
} from "../editor/__fixtures__/plate-corpus";
import { BaseEditorKit } from "../editor/editor-base-kit";
import { LazyPlateStatic, indexEditorPaths } from "./lazy-plate-static";
import {
	EMPTY_STREAMING_STATE,
	parseStreamingMarkdown,
} from "./streaming-markdown-blocks";
import {
	RICH_REMARK_PLUGINS,
	TextEditor,
	getStaticParseWorker,
	safeDeserialize,
	transformSpecialLinks,
} from "./text-editor";

type Node = { text?: string; type?: string; children?: Node[] };

const worker = getStaticParseWorker(BaseEditorKit);

function quietly<T>(run: () => T): T {
	const { error, warn } = console;
	Object.assign(console, { error: () => {}, warn: () => {} });
	try {
		return run();
	} finally {
		Object.assign(console, { error, warn });
	}
}

const parse = (markdown: string) =>
	quietly(
		() =>
			safeDeserialize(worker, markdown, true, RICH_REMARK_PLUGINS) as Node[],
	);

const texts = (nodes: readonly Node[]): string[] =>
	nodes.flatMap((node) =>
		typeof node.text === "string" ? [node.text] : texts(node.children ?? []),
	);

const types = (nodes: readonly Node[]): string[] =>
	nodes.flatMap((node) => [
		...(node.type ? [node.type] : []),
		...types(node.children ?? []),
	]);

describe("markdown code keeps its markup verbatim", () => {
	for (const { title, markdown, verbatim } of CODE_MARKUP_FIXTURES) {
		test(title, () => {
			const settled = texts(parse(markdown));
			for (const text of verbatim) expect(settled).toContain(text);
			const streamed = quietly(
				() =>
					parseStreamingMarkdown(worker, markdown, EMPTY_STREAMING_STATE)
						.blocks,
			);
			expect(streamed as unknown as Node[]).toEqual(parse(markdown));
		});
	}

	test("markup outside code still parses", () => {
		expect(texts(parse("one<br>two"))).toEqual(["one", "\n", "two"]);
	});
});

describe("footnotes", () => {
	const markdown = "A claim.[^1]\n\n[^1]: The **source**.";

	test("preserve linked reference and definition nodes", () => {
		const value = parse(markdown);
		expect(types(value)).toContain("footnoteReference");
		expect(types(value)).toContain("footnoteDefinition");
		expect((value[0] as { children: unknown[] })?.children).toEqual([
			{ text: "A claim." },
			{ type: "footnoteReference", identifier: "1", children: [{ text: "" }] },
		]);
		expect((value[1] as { identifier?: string })?.identifier).toBe("1");
	});

	test("renders links to definitions and back to references", () => {
		const html = renderToStaticMarkup(
			createElement(TextEditor, { initialContent: markdown, isMarkdown: true }),
		);
		expect(html).toContain('aria-label="Footnote 1"');
		expect(html).toContain('aria-label="Back to reference 1"');
		const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map((match) => match[1]);
		const links = [...html.matchAll(/href="#([^"]+)"/g)].map(
			(match) => match[1],
		);
		for (const target of links) expect(ids).toContain(target);
		expect(html).toContain("source");
	});

	test("repeated references have distinct IDs and documents use separate scopes", () => {
		const html = renderToStaticMarkup(
			createElement(
				"div",
				null,
				createElement(TextEditor, {
					initialContent: "First[^1] and again[^1].\n\n[^1]: Source",
					isMarkdown: true,
				}),
				createElement(TextEditor, {
					initialContent: markdown,
					isMarkdown: true,
				}),
			),
		);
		const ids = [...html.matchAll(/\bid="(fn-[^"]+)"/g)].map(
			(match) => match[1],
		);
		expect(new Set(ids).size).toBe(ids.length);
		expect(ids.length).toBe(5);
	});
});

describe("static rendering of a shared value", () => {
	const FROZEN_WRITE = /read.?only|not extensible|frozen/i;

	const deepFreeze = <T,>(value: T): T => {
		if (value && typeof value === "object" && !Object.isFrozen(value)) {
			Object.freeze(value);
			for (const entry of Object.values(value)) deepFreeze(entry);
		}
		return value;
	};

	const documents: ReadonlyArray<readonly [string, () => Value]> = [
		...MARKDOWN_FIXTURES.filter((fixture) => !fixture.usesKatex).map(
			(fixture) =>
				[fixture.id, () => parse(fixture.markdown) as Value] as const,
		),
		...STORED_FIXTURES.filter(
			(fixture) => fixture.nodes.length > 0 && !fixture.usesKatex,
		).map(
			(fixture) =>
				[
					fixture.id,
					() =>
						transformSpecialLinks(
							structuredClone(fixture.nodes) as Node[],
						) as unknown as Value,
				] as const,
		),
	];

	test("never writes to it, even once every node is frozen", () => {
		const writes: string[] = [];
		for (const [id, build] of documents) {
			const value = build();
			const before = JSON.stringify(value);
			deepFreeze(value);
			for (const renderer of [PlateStatic, LazyPlateStatic]) {
				try {
					quietly(() => {
						const editor = createSlateEditor({
							plugins: BaseEditorKit,
							value,
							nodeId: false,
						});
						indexEditorPaths(editor);
						renderToStaticMarkup(createElement(renderer, { editor }));
					});
				} catch (error) {
					const message =
						error instanceof Error ? error.message : String(error);
					if (FROZEN_WRITE.test(message)) writes.push(`${id}: ${message}`);
				}
			}
			if (JSON.stringify(value) !== before) writes.push(`${id}: changed`);
		}
		expect(writes).toEqual([]);
	});
});
