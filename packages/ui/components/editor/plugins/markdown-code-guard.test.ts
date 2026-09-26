import { describe, expect, test } from "bun:test";
import {
	MarkdownPlugin,
	deserializeInlineMd,
	deserializeMd,
} from "@platejs/markdown";
import { createSlateEditor } from "platejs";
import {
	CODE_MARKUP_FIXTURES,
	MARKDOWN_FIXTURES,
} from "../__fixtures__/plate-corpus";
import { BaseEditorKit } from "../editor-base-kit";
import { guardCodeFromJsx } from "./markdown-code-guard";

type Node = { text?: string; children?: Node[] };

const editor = createSlateEditor({ plugins: BaseEditorKit, nodeId: false });
const api = editor.getApi(MarkdownPlugin).markdown;

function quietly<T>(run: () => T): T {
	const { error, warn } = console;
	Object.assign(console, { error: () => {}, warn: () => {} });
	try {
		return run();
	} finally {
		Object.assign(console, { error, warn });
	}
}

const texts = (nodes: readonly unknown[]): string[] =>
	(nodes as readonly Node[]).flatMap((node) =>
		typeof node.text === "string" ? [node.text] : texts(node.children ?? []),
	);

const fixture = (id: string) => {
	const found = MARKDOWN_FIXTURES.find((candidate) => candidate.id === id);
	if (!found) throw new Error(`no markdown fixture ${id}`);
	return found.markdown;
};

const codeBlock = (lang: string, ...lines: string[]) => ({
	children: lines.map((text) => ({
		children: [{ text }],
		type: "code_line",
	})),
	lang,
	type: "code_block",
});

describe("MarkdownKit's deserialize keeps code verbatim", () => {
	for (const { title, markdown, verbatim } of CODE_MARKUP_FIXTURES) {
		test(title, () => {
			const parsed = texts(quietly(() => api.deserialize(markdown)));
			for (const text of verbatim) expect(parsed).toContain(text);
		});

		test(`${title}, block-memoized as the AI chat preview parses`, () => {
			const parsed = texts(
				quietly(() => api.deserialize(markdown, { memoize: true })),
			);
			for (const text of verbatim) expect(parsed).toContain(text);
		});
	}

	test("inline parses, used for copilot accepts and streamed chunks", () => {
		const markdown = 'Use `<div class="a">` and ``<input disabled>``.';
		const verbatim = ['<div class="a">', "<input disabled>"];
		for (const nodes of [
			api.deserializeInline(markdown),
			deserializeInlineMd(editor, markdown),
		]) {
			for (const text of verbatim) expect(texts(nodes)).toContain(text);
		}
	});

	test("the unguarded parser still rewrites code, so the guard is needed", () => {
		const [{ markdown }] = CODE_MARKUP_FIXTURES;
		expect(texts(deserializeMd(editor, markdown))).not.toContain(
			'<div class="a" for="b" checked>x</div>',
		);
	});
});

describe("markup outside code parses as before", () => {
	test("only fixtures with code or escaped tags reach the parser changed", () => {
		expect(
			MARKDOWN_FIXTURES.filter(
				({ markdown }) => guardCodeFromJsx(markdown) !== markdown,
			).map(({ id }) => id),
		).toEqual([
			"X06-embed-urls",
			"X07-map-label",
			"X08-text-that-looks-like-markup",
		]);
	});

	test("HTML in prose still goes through the JSX rewrite", () => {
		const markdown =
			'<u>under</u> <kbd>K</kbd> one<br>two <span style="color: red">red</span>';
		const nodes = api.deserialize(markdown);
		expect(nodes).toEqual(deserializeMd(editor, markdown));
		expect(nodes).toEqual([
			{
				children: [
					{ text: "under", underline: true },
					{ text: " " },
					{ kbd: true, text: "K" },
					{ text: " one" },
					{ text: "\n" },
					{ text: "two " },
					{ color: "red", text: "red" },
				],
				type: "p",
			},
		]);
	});

	test("prose next to code parses as it does alone", () => {
		const prose = "<u>under</u> and <kbd>K</kbd>";
		const nodes = api.deserialize(
			`${prose}\n\n\`\`\`html\n<div class="a"><br></div>\n\`\`\``,
		);
		expect(nodes).toEqual([
			...deserializeMd(editor, prose),
			codeBlock("html", '<div class="a"><br></div>'),
		]);
	});

	test("X06-X08 parse to the values pinned on Plate 49", () => {
		expect(quietly(() => api.deserialize(fixture("X06-embed-urls")))).toEqual([
			codeBlock("embed", "javascript:window.__xss=1"),
			codeBlock(
				"embed",
				'url: "https://example.com/\\"><img src=x onerror=window.__xss=2>"',
			),
		]);
		expect(quietly(() => api.deserialize(fixture("X07-map-label")))).toEqual([
			codeBlock(
				"map",
				'label: "<img src=x onerror=window.__xss=3>"',
				"lat: 1",
				"lng: 2",
			),
		]);
		expect(
			quietly(() =>
				api.deserialize(fixture("X08-text-that-looks-like-markup")),
			),
		).toEqual([
			{
				children: [
					{ text: "Inline " },
					{ code: true, text: "<script>window.__xss=1</script>" },
					{ text: " code." },
				],
				type: "p",
			},
			codeBlock("html", '<img src=x onerror="window.__xss=2">'),
			{
				children: [
					{ text: "Escaped <b onmouseover=window.__xss=3>not a tag</b>" },
				],
				type: "p",
			},
		]);
	});
});
