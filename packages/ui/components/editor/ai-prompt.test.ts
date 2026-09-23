import { describe, expect, test } from "bun:test";
import { type EditorPrompt, getEditorPrompt } from "@platejs/ai";
import { AIChatPlugin, AIPlugin } from "@platejs/ai/react";
import { serializeMd } from "@platejs/markdown";
import { BlockSelectionPlugin } from "@platejs/selection/react";
import { KEYS, type SlateEditor, type TElement, type Value } from "platejs";
import { createPlateEditor } from "platejs/react";
import {
	PROMPT_TEMPLATES,
	renderEditorPrompt,
	submitEditorAI,
	withEditorTemplate,
} from "./ai-prompt";
import { MarkdownKit } from "./plugins/markdown-kit";

const cell = (text: string) => ({
	type: KEYS.td,
	children: [{ type: KEYS.p, children: [{ text }] }],
});

const DOCUMENT: Value = [
	{ id: "title", type: KEYS.h1, children: [{ text: "Title" }] },
	{ id: "first", type: KEYS.p, children: [{ text: "First paragraph." }] },
	{ id: "second", type: KEYS.p, children: [{ text: "Second paragraph." }] },
	{
		id: "grid",
		type: KEYS.table,
		children: [{ type: KEYS.tr, children: [cell("A1"), cell("B1")] }],
	},
];

type Point = { path: number[]; offset: number };

const createEditor = (value: Value = DOCUMENT) =>
	createPlateEditor({
		plugins: [...MarkdownKit, BlockSelectionPlugin, AIPlugin, AIChatPlugin],
		value: structuredClone(value),
	});

type Editor = ReturnType<typeof createEditor>;

const select = (editor: Editor, anchor: Point, focus: Point = anchor) =>
	editor.tf.select({ anchor, focus });

const blockSelect = (editor: Editor, ...ids: string[]) =>
	editor.getApi(BlockSelectionPlugin).blockSelection.set(ids);

/** Plate 49.2.15 `getMarkdown`, transcribed from `@platejs/ai/dist/react/index.mjs`. */
const getMarkdown49 = (
	editor: SlateEditor,
	type: "block" | "editor" | "selection",
) => {
	if (type === "editor") return serializeMd(editor);
	if (type === "block") {
		const blocks = editor.getOption(BlockSelectionPlugin, "isSelectingSome")
			? editor.getApi(BlockSelectionPlugin).blockSelection.getNodes()
			: editor.api.nodes({
					mode: "highest",
					match: (n) => editor.api.isBlock(n),
				});
		const nodes = Array.from(blocks, (entry) => entry[0] as TElement);
		return serializeMd(editor, { value: nodes });
	}
	const fragment = editor.api.fragment<TElement>();
	if (fragment.length === 1) {
		return serializeMd(editor, {
			value: [{ children: fragment[0].children, type: KEYS.p }],
		});
	}
	return serializeMd(editor, { value: fragment });
};

/** Plate 49.2.15 `replacePlaceholders`, transcribed from the same file. */
const replacePlaceholders49 = (
	editor: SlateEditor,
	text: string,
	prompt: string,
) => {
	let result = text.replace("{prompt}", prompt || "");
	const placeholders = {
		"{block}": "block",
		"{editor}": "editor",
		"{selection}": "selection",
	} as const;
	for (const [placeholder, type] of Object.entries(placeholders)) {
		if (result.includes(placeholder)) {
			result = result.replace(placeholder, getMarkdown49(editor, type));
		}
	}
	return result;
};

const CONTINUE_EMPTY = `<Document>
{editor}
</Document>
Start writing a new paragraph AFTER <Document> ONLY ONE SENTENCE`;

const MENU_PROMPTS: readonly EditorPrompt[] = [
	"Continue writing AFTER <Block> ONLY ONE SENTENCE. DONT REPEAT THE TEXT.",
	CONTINUE_EMPTY,
	{ default: "Summarize {editor}", selecting: "Summarize" },
	{ default: "Explain {editor}", selecting: "Explain" },
	"Improve the writing",
	"Emojify",
	"Make longer",
	"Make shorter",
	"Fix spelling and grammar",
	"Simplify the language",
	"Summarize this content as bullet points",
	"what does {block} mean?",
];

const SELECTION_STATES: ReadonlyArray<
	readonly [string, (editor: Editor) => void, string]
> = [
	[
		"a collapsed cursor",
		(editor) => select(editor, { path: [1, 0], offset: 5 }),
		PROMPT_TEMPLATES.userDefault,
	],
	[
		"a cursor in a table cell",
		(editor) => select(editor, { path: [3, 0, 1, 0, 0], offset: 1 }),
		PROMPT_TEMPLATES.userDefault,
	],
	[
		"a selection inside one block",
		(editor) =>
			select(editor, { path: [1, 0], offset: 0 }, { path: [1, 0], offset: 5 }),
		PROMPT_TEMPLATES.userSelecting,
	],
	[
		"a selection across blocks",
		(editor) =>
			select(editor, { path: [0, 0], offset: 2 }, { path: [2, 0], offset: 6 }),
		PROMPT_TEMPLATES.userSelecting,
	],
	[
		"a block selection",
		(editor) => {
			select(editor, { path: [1, 0], offset: 0 });
			blockSelect(editor, "second", "grid");
		},
		PROMPT_TEMPLATES.userBlockSelecting,
	],
];

describe("editor prompt rendering matches Plate 49", () => {
	for (const [label, arrange, template] of SELECTION_STATES) {
		test(`every menu prompt with ${label}`, () => {
			const editor = createEditor();
			arrange(editor);

			for (const prompt of MENU_PROMPTS) {
				const resolved = getEditorPrompt(editor, { prompt });
				expect(
					getEditorPrompt(editor, { prompt: withEditorTemplate(prompt) }),
				).toBe(replacePlaceholders49(editor, template, resolved));
			}
		});
	}

	test("picks the template from the selection state at submit time", () => {
		const editor = createEditor();
		const render = () =>
			getEditorPrompt(editor, { prompt: withEditorTemplate("Improve") });

		select(editor, { path: [1, 0], offset: 3 });
		expect(render()).toEndWith("</Reminder>\nImprove");
		expect(render()).toContain("<Block>\nFirst paragraph.\n\n</Block>");
		expect(render()).not.toContain("<Selection>\n");

		select(editor, { path: [1, 0], offset: 0 }, { path: [1, 0], offset: 5 });
		expect(render()).toEndWith("Improve about <Selection>");
		expect(render()).toContain(
			"<Block>\nFirst paragraph.\n\n</Block>\n<Selection>\nFirst\n\n</Selection>",
		);

		blockSelect(editor, "second");
		expect(render()).toContain(
			"<Selection>\nSecond paragraph.\n\n</Selection>",
		);
		expect(render()).not.toContain("<Block>\n");
	});
});

describe("renderEditorPrompt placeholders", () => {
	test("{block} is the highest block around the cursor, so a table cell yields its table", () => {
		const editor = createEditor();
		select(editor, { path: [3, 0, 1, 0, 0], offset: 1 });

		const block = renderEditorPrompt(editor, "{block}", "");

		expect(block).toBe(serializeMd(editor, { value: [DOCUMENT[3]] }));
		expect(block).toContain("| A1 | B1 |");
	});

	test("{block} is the block-selected nodes while block selecting", () => {
		const editor = createEditor();
		select(editor, { path: [0, 0], offset: 0 });
		blockSelect(editor, "first", "second");

		expect(renderEditorPrompt(editor, "{block}", "")).toBe(
			"First paragraph.\n\nSecond paragraph.\n",
		);
	});

	test("{selection} inside one block is a bare paragraph, across blocks the fragment", () => {
		const editor = createEditor();
		select(editor, { path: [0, 0], offset: 0 }, { path: [0, 0], offset: 3 });
		expect(renderEditorPrompt(editor, "{selection}", "")).toBe("Tit\n");

		select(editor, { path: [0, 0], offset: 2 }, { path: [1, 0], offset: 5 });
		expect(renderEditorPrompt(editor, "{selection}", "")).toBe(
			"# tle\n\nFirst\n",
		);
	});

	test("{editor} is the whole document", () => {
		const editor = createEditor();
		select(editor, { path: [1, 0], offset: 0 });

		expect(renderEditorPrompt(editor, "<Document>\n{editor}", "")).toBe(
			`<Document>\n${serializeMd(editor)}`,
		);
	});

	test("{prompt} goes in first, so placeholders inside the prompt resolve", () => {
		const editor = createEditor();
		select(editor, { path: [1, 0], offset: 0 });

		expect(renderEditorPrompt(editor, "Q: {prompt}", "Explain {editor}")).toBe(
			`Q: Explain ${serializeMd(editor)}`,
		);
	});

	test("each placeholder resolves at its first occurrence only", () => {
		const editor = createEditor();
		select(editor, { path: [1, 0], offset: 0 });

		expect(
			renderEditorPrompt(editor, "{block}|{block}|{prompt}", "{prompt}"),
		).toBe("First paragraph.\n|{block}|{prompt}");
	});

	test("an empty template input leaves no placeholder text behind", () => {
		const editor = createEditor();
		select(editor, { path: [1, 0], offset: 0 });

		expect(renderEditorPrompt(editor, "a{prompt}b", "")).toBe("ab");
	});
});

describe("renderEditorPrompt fixes Plate 49 replacement bugs", () => {
	test("$& and $$ in the typed prompt reach the model verbatim", () => {
		const editor = createEditor();
		select(editor, { path: [1, 0], offset: 0 });
		const prompt = "Price it in $$ and keep $& literal";

		expect(renderEditorPrompt(editor, "P: {prompt}", prompt)).toBe(
			`P: ${prompt}`,
		);
		expect(replacePlaceholders49(editor, "P: {prompt}", prompt)).toBe(
			"P: Price it in $ and keep {prompt} literal",
		);
	});

	test("$$ math in the document reaches the model verbatim", () => {
		const math: Value = [
			{
				id: "math",
				type: KEYS.equation,
				texExpression: "x^2",
				children: [{ text: "" }],
			},
			{ id: "after", type: KEYS.p, children: [{ text: "after" }] },
		];
		const editor = createEditor(math);
		select(editor, { path: [1, 0], offset: 0 });
		const markdown = serializeMd(editor);
		expect(markdown).toContain("$$\nx^2\n$$");

		expect(renderEditorPrompt(editor, "{editor}", "")).toBe(markdown);
		expect(replacePlaceholders49(editor, "{editor}", "")).not.toBe(markdown);
	});

	test("placeholder text inside the document is never expanded", () => {
		const code: Value = [
			{
				id: "code",
				type: KEYS.codeBlock,
				children: [{ type: KEYS.codeLine, children: [{ text: "{editor}" }] }],
			},
		];
		const editor = createEditor(code);
		select(editor, { path: [0, 0, 0], offset: 0 });
		const block = serializeMd(editor, { value: [code[0]] });
		expect(block).toContain("{editor}");

		expect(renderEditorPrompt(editor, "{block}", "")).toBe(block);
		expect(replacePlaceholders49(editor, "{block}", "")).not.toBe(block);
	});
});

type Sent = { text: string; options: unknown };

const withStubChat = (editor: Editor) => {
	const sent: Sent[] = [];
	editor.setOption(AIChatPlugin, "chat", {
		messages: [],
		sendMessage: async (message: { text: string }, options: unknown) => {
			sent.push({ text: message.text, options });
		},
		setMessages: () => {},
		status: "ready",
		stop: async () => {},
	} as never);
	return sent;
};

describe("submitEditorAI", () => {
	test("sends the templated prompt as a generate request in the requested mode", () => {
		const editor = createEditor();
		const sent = withStubChat(editor);
		select(editor, { path: [1, 0], offset: 16 });

		expect(submitEditorAI(editor, "Continue", { mode: "insert" })).toBe(true);

		expect(sent).toHaveLength(1);
		expect(sent[0].text).toBe(
			replacePlaceholders49(editor, PROMPT_TEMPLATES.userDefault, "Continue"),
		);
		expect(editor.getOption(AIChatPlugin, "mode")).toBe("insert");
		expect(editor.getOption(AIChatPlugin, "toolName")).toBe("generate");
	});

	test("defaults to chat mode for a selection and insert mode at a cursor", () => {
		const editor = createEditor();
		withStubChat(editor);

		select(editor, { path: [1, 0], offset: 0 }, { path: [1, 0], offset: 5 });
		submitEditorAI(editor, "Improve the writing");
		expect(editor.getOption(AIChatPlugin, "mode")).toBe("chat");

		select(editor, { path: [1, 0], offset: 3 });
		submitEditorAI(editor, "Improve the writing");
		expect(editor.getOption(AIChatPlugin, "mode")).toBe("insert");
	});

	test("an empty prompt sends nothing", () => {
		const editor = createEditor();
		const sent = withStubChat(editor);
		select(editor, { path: [1, 0], offset: 0 });

		expect(submitEditorAI(editor, "")).toBe(false);
		expect(sent).toEqual([]);
	});
});
