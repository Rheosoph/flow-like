import { describe, expect, setDefaultTimeout, test } from "bun:test";
import { Window } from "happy-dom";
import type { Descendant, Value } from "platejs";
import type { PlateEditor } from "platejs/react";
import { act, createElement } from "react";

setDefaultTimeout(60_000);

const window = new Window({
	url: "http://localhost:3000/",
	settings: {
		disableJavaScriptFileLoading: true,
		disableCSSFileLoading: true,
		disableIframePageLoading: true,
		handleDisabledFileLoadingAsSuccess: true,
	},
});
Object.assign(window, { SyntaxError, TypeError, Error });
// happy-dom leaves compatMode undefined; KaTeX then refuses katex.render() as quirks mode.
Object.defineProperty(window.document, "compatMode", { value: "CSS1Compat" });

/** Everything Plate, Radix, dnd, KaTeX and DOMPurify touch. */
const FORWARDED_GLOBALS = [
	"document",
	"navigator",
	"location",
	"localStorage",
	"sessionStorage",
	"Node",
	"Element",
	"HTMLElement",
	"HTMLDivElement",
	"HTMLSpanElement",
	"HTMLAnchorElement",
	"HTMLImageElement",
	"HTMLVideoElement",
	"HTMLAudioElement",
	"HTMLInputElement",
	"HTMLTextAreaElement",
	"HTMLButtonElement",
	"HTMLIFrameElement",
	"HTMLTemplateElement",
	"HTMLFormElement",
	"HTMLCanvasElement",
	"SVGElement",
	"Text",
	"Comment",
	"Document",
	"DocumentFragment",
	"ShadowRoot",
	"Range",
	"Selection",
	"DOMParser",
	"XMLSerializer",
	"Event",
	"CustomEvent",
	"UIEvent",
	"KeyboardEvent",
	"MouseEvent",
	"PointerEvent",
	"FocusEvent",
	"InputEvent",
	"ClipboardEvent",
	"DragEvent",
	"MutationObserver",
	"ResizeObserver",
	"IntersectionObserver",
	"NodeFilter",
	"TreeWalker",
	"File",
	"FileList",
	"Blob",
	"DataTransfer",
	"CSSStyleDeclaration",
	"HTMLCollection",
	"NodeList",
	"DOMRect",
	"Image",
	"matchMedia",
	"getComputedStyle",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"getSelection",
	"customElements",
	"devicePixelRatio",
] as const;

/** Installed before react-dom and DOMPurify load: both probe the DOM at import. */
{
	const source = window as unknown as Record<string, unknown>;
	const globals: Record<string, unknown> = { window };
	for (const name of FORWARDED_GLOBALS) {
		const value = source[name];
		if (value === undefined) continue;
		globals[name] =
			typeof value === "function" && /^[a-z]/.test(name)
				? (value as (...args: unknown[]) => unknown).bind(window)
				: value;
	}
	Object.assign(globalThis, globals, { IS_REACT_ACT_ENVIRONMENT: true });
}

/** Markup assigned to `innerHTML` of an element owned by the live document. */
const liveMarkupWrites: string[] = [];
{
	const prototype = window.Element.prototype;
	const descriptor = Object.getOwnPropertyDescriptor(prototype, "innerHTML");
	const setter = descriptor?.set;
	if (!descriptor || !setter) throw new Error("happy-dom has no innerHTML");
	Object.defineProperty(prototype, "innerHTML", {
		...descriptor,
		set(this: { ownerDocument: unknown }, markup: string) {
			if (this.ownerDocument === window.document)
				liveMarkupWrites.push(String(markup));
			setter.call(this, markup);
		},
	});
}

const { createRoot } = await import("react-dom/client");
const { renderToStaticMarkup } = await import("react-dom/server");
const { KEYS, createSlateEditor } = await import("platejs");
const { PlateStatic } = await import("platejs/static");
const { Plate, createPlateEditor } = await import("platejs/react");
const { createEditorKit } = await import("./editor-kit");
const { BaseEditorKit } = await import("./editor-base-kit");
const { Editor, EditorContainer } = await import("./ui/editor");
const { deserializeHtmlFile } = await import("./ui/import-toolbar-button");
const { insertBlock, setBlockType, toggleBlockAt } = await import(
	"./transforms"
);
const { HTML_IMPORT_FIXTURES, MARKDOWN_FIXTURES, STORED_FIXTURES } =
	await import("./__fixtures__/plate-corpus");
const { RICH_REMARK_PLUGINS, getStaticParseWorker, safeDeserialize } =
	await import("../ui/text-editor");

const KIT = createEditorKit();

/** Lowlight, KaTeX quirks mode and the copilot's missing backend log noise. */
async function quietly<T>(run: () => T | Promise<T>): Promise<T> {
	const { error, warn, log, dir } = console;
	const mute = () => {};
	Object.assign(console, { error: mute, warn: mute, log: mute, dir: mute });
	try {
		return await run();
	} finally {
		Object.assign(console, { error, warn, log, dir });
	}
}

/**
 * `h1<"Title">`, `p[decimal:3]<"x">`, `a(url)<"text">`, `"bold"{bold}`: node
 * types, list props, link urls and marks, without ids.
 */
function outline(nodes: readonly Descendant[]): string {
	const describeNode = (node: Descendant): string => {
		const record = node as unknown as Record<string, unknown>;
		if (typeof record.text === "string") {
			const marks = Object.keys(record)
				.filter((key) => key !== "text")
				.sort();
			return `${JSON.stringify(record.text)}${marks.length ? `{${marks.join(",")}}` : ""}`;
		}
		const list =
			typeof record.listStyleType === "string"
				? `[${record.listStyleType}${record.listStart ? `:${record.listStart}` : ""}${
						typeof record.checked === "boolean"
							? ` checked=${record.checked}`
							: ""
					}]`
				: "";
		const url = typeof record.url === "string" ? `(${record.url})` : "";
		const children = (record.children as Descendant[])
			.map(describeNode)
			.join(" ");
		return `${record.type}${url}${list}<${children}>`;
	};
	return nodes.map(describeNode).join(" | ");
}

/** The app's editable surface: fixed toolbar, floating toolbars, block handles. */
async function mountEditor(editor: PlateEditor) {
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	const root = createRoot(host as unknown as HTMLElement);
	await quietly(async () => {
		await act(async () => {
			root.render(
				<Plate editor={editor}>
					<EditorContainer>
						<Editor variant="none" />
					</EditorContainer>
				</Plate>,
			);
		});
	});
	return {
		host: host as unknown as HTMLElement,
		unmount: () =>
			quietly(async () => {
				await act(async () => root.unmount());
				host.remove();
			}),
	};
}

const paragraph = (text = ""): Value => [{ type: "p", children: [{ text }] }];
const codeBlock = (text = ""): Value => [
	{
		type: "code_block",
		children: [{ type: "code_line", children: [{ text }] }],
	},
];

function createEditor(value: Value, at: "start" | "end" = "end") {
	const editor = createPlateEditor({
		plugins: KIT,
		value: structuredClone(value),
	});
	const point = at === "start" ? editor.api.start([0]) : editor.api.end([0]);
	if (!point) throw new Error("the document has no first block");
	editor.tf.select(point);
	return editor;
}

/** Types one character at a time; ⏎ = Enter, ⌫ = Backspace, ↶ = undo. */
function typeKeys(
	keys: string,
	value: Value = paragraph(),
	at: "start" | "end" = "end",
) {
	const editor = createEditor(value, at);
	for (const key of Array.from(keys)) {
		if (key === "⏎") editor.tf.insertBreak();
		else if (key === "⌫") editor.tf.deleteBackward("character");
		else if (key === "↶") editor.undo();
		else editor.tf.insertText(key);
	}
	return outline(editor.children);
}

const clipboard = (data: Record<string, string>) =>
	({
		getData: (type: string) => data[type] ?? "",
		setData: () => {},
		types: Object.keys(data),
		files: [],
		items: [],
	}) as unknown as DataTransfer;

function paste(
	text: string,
	value: Value = paragraph(),
	select?: "block",
): string {
	const editor = createEditor(value);
	if (select === "block") {
		const [start, end] = [editor.api.start([0]), editor.api.end([0])];
		if (!start || !end) throw new Error("the document has no first block");
		editor.tf.select({ anchor: start, focus: end });
	}
	editor.tf.insertData(clipboard({ "text/plain": text }));
	return outline(editor.children);
}

type ShortcutCase = readonly [
	keys: string,
	expected: string,
	value?: Value,
	at?: "start" | "end",
];

const check = (cases: readonly ShortcutCase[]) => {
	for (const [keys, expected, value, at] of cases) {
		test(JSON.stringify(keys), () => {
			expect(typeKeys(keys, value, at)).toBe(expected);
		});
	}
};

/** The trailing `p` is TrailingBlockPlugin's, as on Plate 49. */
describe("markdown block shortcuts", () => {
	check([
		["# Title", 'h1<"Title"> | p<"">'],
		["## Title", 'h2<"Title"> | p<"">'],
		["### Title", 'h3<"Title"> | p<"">'],
		["#Title", 'p<"#Title">'],
		["a # b", 'p<"a # b">'],
		["> quote", 'blockquote<p<"quote">> | p<"">'],
		["x > y", 'p<"x > y">'],
		["```", 'code_block<code_line<"">> | p<"">'],
		["```x", 'code_block<code_line<"x">> | p<"">'],
		["---", 'hr<""> | p<""> | p<"">'],
		["---x", 'hr<""> | p<"x"> | p<"">'],
		["—-", 'hr<""> | p<""> | p<"">'],
		["___ ", 'hr<""> | p<""> | p<"">'],
		["- item", 'p[disc]<"item">'],
		["* item", 'p[disc]<"item">'],
		["a * b", 'p<"a * b">'],
		["1. item", 'p[decimal]<"item">'],
		["3. item", 'p[decimal:3]<"item">'],
		["12. item", 'p[decimal:12]<"item">'],
		["1) item", 'p[decimal]<"item">'],
		["[] todo", 'p[todo checked=false]<"todo">'],
		["[x] done", 'p[todo checked=true]<"done">'],
	]);

	describe("on existing text", () => {
		check([
			["# ", 'h1<"Title"> | p<"">', paragraph("Title"), "start"],
			["- ", 'p[disc]<"Item">', paragraph("Item"), "start"],
			["> ", 'blockquote<p<"Q">> | p<"">', paragraph("Q"), "start"],
			[
				"```",
				'p<"code"> | code_block<code_line<"">> | p<"">',
				paragraph("code"),
				"start",
			],
		]);
	});

	describe("inside other blocks", () => {
		const h1: Value = [{ type: "h1", children: [{ text: "" }] }];
		const bullet: Value = [
			{ type: "p", indent: 1, listStyleType: "disc", children: [{ text: "" }] },
		];
		const quote: Value = [
			{
				type: "blockquote",
				children: [{ type: "p", children: [{ text: "" }] }],
			},
		];
		check([
			["# x", 'h1<"# x"> | p<"">', h1],
			["- x", 'h1[disc]<"x"> | p<"">', h1],
			["```", 'code_block<code_line<"">> | p<"">', h1],
			["---", 'hr<""> | p<""> | p<"">', h1],
			["# x", 'h1[disc]<"x"> | p<"">', bullet],
			["- x", 'p<"x">', bullet],
			["> x", 'blockquote<p<"> x">> | p<"">', quote],
		]);
	});

	describe("never inside code blocks", () => {
		check([
			[
				"**x** -> (c) # a",
				'code_block<code_line<"**x** -> (c) # a">> | p<"">',
				codeBlock(),
			],
			["```", 'code_block<code_line<"```">> | p<"">', codeBlock()],
			["- x", 'code_block<code_line<"- x">> | p<"">', codeBlock()],
		]);
	});

	describe("deliberate changes from Plate 49", () => {
		// 49 turned these into h4-h6 nodes the editor kits never register (rendered as a plain div).
		check([
			["#### Title", 'p<"#### Title">'],
			["###### Title", 'p<"###### Title">'],
		]);

		// 49 converted the whole block, hiding the text after the cursor inside the hr void.
		check([["---", 'p<"—-after">', paragraph("after"), "start"]]);

		// 49 restarted "N)" lists at 1 (it parsed "2)" as NaN); "N." already started at N.
		check([["2) x", 'p[decimal:2]<"x">']]);

		// 49 checked "__*" first and left the asterisks: italic+underline "*ub*".
		check([["__**ub**__", 'p<"ub"{bold,underline}>']]);
	});
});

describe("markdown mark shortcuts", () => {
	check([
		["**bold** ", 'p<"bold"{bold} " ">'],
		["**bold**", 'p<"bold"{bold}>'],
		["*it* ", 'p<"it"{italic} " ">'],
		["_it_ ", 'p<"it"{italic} " ">'],
		["__u__", 'p<"u"{underline}>'],
		["__bold__ ", 'p<"bold"{underline} " ">'],
		["~~s~~ ", 'p<"s"{strikethrough} " ">'],
		["`c` ", 'p<"c"{code} " ">'],
		["^s^ ", 'p<"s"{superscript} " ">'],
		["~s~ ", 'p<"s"{subscript} " ">'],
		["==h== ", 'p<"h"{highlight} " ">'],
		["≡h≡ ", 'p<"h"{highlight} " ">'],
		["***bi***", 'p<"bi"{bold,italic}>'],
		["__*ui*__", 'p<"ui"{italic,underline}>'],
		["___***ubi***___", 'p<"ubi"{bold,italic,underline}>'],
		["say **hi** now", 'p<"say " "hi"{bold} " now">'],
		["**a b** ", 'p<"a b"{bold} " ">'],
		["**a** *b* ", 'p<"a"{bold} " " "b"{italic} " ">'],
		["**a->b** ", 'p<"a→b"{bold} " ">'],
		["a**b**", 'p<"a**b**">'],
		["** b** ", 'p<"** b** ">'],
	]);
});

describe("text substitutions", () => {
	check([
		['a "q" b', 'p<"a “q” b">'],
		["a 'q' b", 'p<"a ‘q’ b">'],
		["don't", `p<"don't">`],
		["wait...", 'p<"wait…">'],
		["a -- b", 'p<"a — b">'],
		[">> x << y", 'p<"» x « y">'],
		["(tm) (TM) (r) (R) (c) (C)", 'p<"™ ™ ® ® © ©">'],
		["&trade; &reg; &copy; &sect;", 'p<"™ ® © §">'],
		["-> <- => <= ≤=", 'p<"→ ← ⇒ ⇐ ⇐">'],
		["!> !< >= !>= !<=", 'p<"≯ ≮ ≥ ≯= ≮=">'],
		["!= == !== ~= !~=", 'p<"≠ ≡ ≢ ≈ !≈">'],
		["+- %% %%%", 'p<"± ‰ ‱">'],
		[
			"1/2 1/3 1/4 1/5 1/6 1/7 1/8 1/9 1/10 2/3 2/5 3/4 3/5 3/8 4/5 5/6 5/8 7/8",
			'p<"½ ⅓ ¼ ⅕ ⅙ ⅐ ⅛ ⅑ ⅒ ⅔ ⅖ ¾ ⅗ ⅜ ⅘ ⅚ ⅝ ⅞">',
		],
		[
			"^o ^+ ^- ^0 ^1 ^2 ^3 ^4 ^5 ^6 ^7 ^8 ^9",
			'p<"° ⁺ ⁻ ⁰ ¹ ² ³ ⁴ ⁵ ⁶ ⁷ ⁸ ⁹">',
		],
		["~+ ~- ~0 ~1 ~2 ~3 ~4 ~5 ~6 ~7 ~8 ~9", 'p<"₊ ₋ ₀ ₁ ₂ ₃ ₄ ₅ ₆ ₇ ₈ ₉">'],
		["11/2", 'p<"1½">'],
		// 49 also turned "//" into "÷", which broke every typed URL.
		["a//b", 'p<"a//b">'],
		["https://x.dev", 'p<"https://x.dev">'],
	]);

	describe("Backspace right after a symbol restores what was typed", () => {
		check([
			["->⌫", 'p<"->">'],
			["(c)⌫", 'p<"(c)">'],
			["...⌫", 'p<"...">'],
			["a --⌫", 'p<"a --">'],
			["1/2⌫", 'p<"1/2">'],
			["a ==⌫", 'p<"a ==">'],
			["a → b⌫⌫⌫", 'p<"a ->">'],
			['a "q"⌫', 'p<"a “q">'],
			["abc⌫", 'p<"ab">'],
		]);
	});

	describe("undo reverts a conversion", () => {
		check([
			["# x↶", 'p<"">'],
			["**b**↶", 'p<"">'],
			["->↶", 'p<"">'],
			["- ↶", 'p<"">'],
		]);
	});
});

describe("autolink", () => {
	check([
		[
			"see https://platejs.org ",
			'p<"see " a(https://platejs.org)<"https://platejs.org"> " ">',
		],
		[
			"see https://platejs.org⏎",
			'p<"see " a(https://platejs.org)<"https://platejs.org"> ""> | p<"">',
		],
		["mailto:a@b.co ", 'p<"" a(mailto:a@b.co)<"mailto:a@b.co"> " ">'],
		["www.example.com ", 'p<"www.example.com ">'],
		["example.com ", 'p<"example.com ">'],
		["javascript:void x", 'p<"javascript:void x">'],
		// 49 linked inside code blocks.
		[
			"https://a.dev ",
			'code_block<code_line<"https://a.dev ">> | p<"">',
			codeBlock(),
		],
	]);

	describe("URLs the link plugin refuses stay text", () => {
		// The stock rule selected the URL and swallowed the key; the next one replaced it.
		check([
			["ftp://x.io x", 'p<"ftp://x.io x">'],
			["ssh://git@x.io x", 'p<"ssh://git@x.io x">'],
			["ftp://x.io⏎x", 'p<"ftp://x.io"> | p<"x">'],
		]);
	});

	describe("on paste", () => {
		test("a URL becomes a link", () => {
			expect(paste("https://platejs.org/docs")).toBe(
				'p<"" a(https://platejs.org/docs)<"https://platejs.org/docs"> "">',
			);
			expect(paste("mailto:a@b.co")).toBe(
				'p<"" a(mailto:a@b.co)<"mailto:a@b.co"> "">',
			);
		});

		test("over a selection it links the selected text", () => {
			expect(
				paste("https://platejs.org", paragraph("select me"), "block"),
			).toBe('p<"" a(https://platejs.org)<"select me"> "">');
		});

		test("text, script URLs, paths and anchors stay text", async () => {
			await quietly(() => {
				expect(paste("hello world")).toBe('p<"hello world">');
				expect(paste("javascript:alert(1)")).toBe('p<"javascript:alert(1)">');
				expect(paste("/docs/page")).toBe('p<"/docs/page">');
				expect(paste("#section")).toBe('p<"#section">');
			});
		});

		test("inside a code block it stays text", () => {
			expect(paste("https://platejs.org", codeBlock())).toBe(
				'code_block<code_line<"https://platejs.org">> | p<"">',
			);
		});
	});
});

describe("legacy flat blockquotes", () => {
	const flatQuote: Value = [
		{
			type: "blockquote",
			children: [
				{ text: "Quote " },
				{ text: "bold", bold: true },
				{ type: "a", url: "https://x.io", children: [{ text: "l" }] },
				{ text: "" },
			],
		},
	];

	test("keep every typed character while becoming containers", () => {
		expect(typeKeys(" more", flatQuote)).toBe(
			'blockquote<p<"Quote " "bold"{bold} a(https://x.io)<"l"> " more">> | p<"">',
		);
	});

	test("typing '> ' inside one does not nest", () => {
		expect(
			typeKeys("> x", [{ type: "blockquote", children: [{ text: "" }] }]),
		).toBe('blockquote<p<"> x">> | p<"">');
	});
});

/** Plate 49 quotes were flat, so "Turn into" replaced the quote itself. */
describe("block transforms on blockquote containers", () => {
	const quote = (...texts: string[]): Value => [
		{
			type: "blockquote",
			children: texts.map((text) => ({ type: "p", children: [{ text }] })),
		},
	];

	const inQuote = (
		value: Value,
		paragraph: number,
		transform: (editor: PlateEditor) => void,
	) => {
		const editor = createEditor(value);
		editor.tf.select({ path: [0, paragraph, 0], offset: 0 });
		transform(editor);
		return outline(editor.children);
	};

	describe("turn-into toolbar", () => {
		test("Text lifts the paragraph out of the quote", () => {
			expect(inQuote(quote("Q"), 0, (e) => setBlockType(e, KEYS.p))).toBe(
				'p<"Q">',
			);
		});

		test("Heading lifts it out as a heading", () => {
			expect(inQuote(quote("Q"), 0, (e) => setBlockType(e, KEYS.h1))).toBe(
				'h1<"Q"> | p<"">',
			);
		});

		test("Bulleted list lifts it out as a list item", () => {
			expect(inQuote(quote("Q"), 0, (e) => setBlockType(e, KEYS.ul))).toBe(
				'p[disc]<"Q">',
			);
		});

		test("only the paragraph with the caret leaves a longer quote", () => {
			expect(
				inQuote(quote("a", "b", "c"), 1, (e) => setBlockType(e, KEYS.p)),
			).toBe('blockquote<p<"a">> | p<"b"> | blockquote<p<"c">> | p<"">');
		});

		test("Quote inside a quote does not nest", () => {
			expect(
				inQuote(quote("Q"), 0, (e) => setBlockType(e, KEYS.blockquote)),
			).toBe('blockquote<p<"Q">>');
		});

		test("Quote on a paragraph wraps it", () => {
			const editor = createEditor(paragraph("a"));
			setBlockType(editor, KEYS.blockquote);
			expect(outline(editor.children)).toBe('blockquote<p<"a">> | p<"">');
		});
	});

	describe("block menu", () => {
		const turnFirstBlockInto = (value: Value, type: string) => {
			const editor = createEditor(value);
			toggleBlockAt(editor, type, [0]);
			return outline(editor.children);
		};

		test("Paragraph unwraps the quote", () => {
			expect(turnFirstBlockInto(quote("a", "b"), KEYS.p)).toBe(
				'p<"a"> | p<"b">',
			);
		});

		test("Heading unwraps it into headings", () => {
			expect(turnFirstBlockInto(quote("a", "b"), KEYS.h2)).toBe(
				'h2<"a"> | h2<"b"> | p<"">',
			);
		});

		test("Blockquote toggles the quote off", () => {
			expect(turnFirstBlockInto(quote("a"), KEYS.blockquote)).toBe('p<"a">');
		});

		test("Blockquote on a paragraph quotes it", () => {
			expect(turnFirstBlockInto(paragraph("a"), KEYS.blockquote)).toBe(
				'blockquote<p<"a">> | p<"">',
			);
		});

		test("a legacy flat quote still toggles like Plate 49", () => {
			expect(
				turnFirstBlockInto(
					[{ type: "blockquote", children: [{ text: "Q" }] }],
					KEYS.p,
				),
			).toBe('p<"Q">');
		});
	});

	test("inserting a quote from inside a quote adds one after it", () => {
		expect(inQuote(quote("Q"), 0, (e) => insertBlock(e, KEYS.blockquote))).toBe(
			'blockquote<p<"Q">> | blockquote<p<"">> | p<"">',
		);
	});
});

describe("Backspace at the start of a heading", () => {
	const backspaceAtSecondBlock = (value: Value) => {
		const editor = createEditor(value);
		editor.tf.select({ path: [1, 0], offset: 0 });
		editor.tf.deleteBackward("character");
		return outline(editor.children);
	};

	test("merges it into the block above, as on Plate 49", () => {
		expect(
			backspaceAtSecondBlock([
				{ type: "p", children: [{ text: "Intro " }] },
				{ type: "h2", children: [{ text: "Title" }] },
			]),
		).toBe('p<"Intro Title"> | p<"">');
	});

	test("removes an empty block above and keeps the heading", () => {
		expect(
			backspaceAtSecondBlock([
				{ type: "p", children: [{ text: "" }] },
				{ type: "h1", children: [{ text: "Title" }] },
			]),
		).toBe('h1<"Title"> | p<"">');
	});

	test("Enter at the end still continues with a paragraph", () => {
		expect(typeKeys("⏎x", [{ type: "h3", children: [{ text: "T" }] }])).toBe(
			'h3<"T"> | p<"x"> | p<"">',
		);
	});
});

describe("date chips", () => {
	test("the caret steps over them, as on Plate 49", () => {
		const editor = createEditor([
			{
				type: "p",
				children: [
					{ text: "a" },
					{ type: "date", date: "2024-01-15", children: [{ text: "" }] },
					{ text: "b" },
				],
			},
		]);
		editor.tf.select({ path: [0, 0], offset: 1 });
		editor.tf.move({ unit: "character" });

		expect(editor.selection?.anchor).toEqual({ path: [0, 2], offset: 0 });
	});
});

describe("the mounted editor", () => {
	test("applies shortcuts typed into the rendered editor", async () => {
		const editor = createPlateEditor({ plugins: KIT, value: paragraph() });
		const view = await mountEditor(editor);
		const end = editor.api.end([0]);
		if (!end) throw new Error("the document has no first block");
		await act(async () => editor.tf.select(end));
		await quietly(async () => {
			await act(async () => {
				for (const key of Array.from("# Title⏎- item⏎⏎**bold** -> ")) {
					if (key === "⏎") editor.tf.insertBreak();
					else editor.tf.insertText(key);
				}
			});
		});

		const content = view.host.querySelector('[data-slate-editor="true"]');
		const markup = {
			heading: content?.querySelector("h1")?.textContent,
			listItem: content?.querySelector("ul li")?.textContent,
			bold: content?.querySelector("strong")?.textContent,
			arrow: content?.textContent?.includes("→"),
		};
		await view.unmount();

		expect(markup).toEqual({
			heading: "Title",
			listItem: "item",
			bold: "bold",
			arrow: true,
		});
	});
});

describe("equations with non-string texExpression (GHSA-p8g2-cf33-p28j)", () => {
	const equations = (texExpression: unknown): Value => [
		{ type: "equation", texExpression, children: [{ text: "" }] } as never,
		{
			type: "p",
			children: [
				{ text: "inline " },
				{
					type: "inline_equation",
					texExpression,
					children: [{ text: "" }],
				} as never,
				{ text: "" },
			],
		},
	];

	const renderStatic = (value: Value) =>
		quietly(() =>
			renderToStaticMarkup(
				createElement(PlateStatic, {
					editor: createSlateEditor({
						plugins: BaseEditorKit,
						value,
						nodeId: false,
					}),
				}),
			),
		);

	const texAnnotations = (markup: string) =>
		Array.from(
			markup.matchAll(
				/<annotation encoding="application\/x-tex">([^<]*)<\/annotation>/g,
			),
			(match) => match[1],
		);

	for (const [label, texExpression, text] of [
		["a number", 42, "42"],
		["a boolean", true, "true"],
	] as const) {
		test(`${label} renders as its text`, async () => {
			const markup = await renderStatic(equations(texExpression));
			expect(texAnnotations(markup)).toEqual([text, text]);
			expect(markup).not.toContain("bg-muted p-3 pr-9");
		});
	}

	for (const [label, texExpression] of [
		["missing", undefined],
		["null", null],
		["an object", { toString: "x" }],
	] as const) {
		test(`${label} renders the empty placeholders`, async () => {
			const markup = await renderStatic(equations(texExpression));
			expect(texAnnotations(markup)).toEqual([""]);
			expect(markup).toContain("bg-muted p-3 pr-9");
			expect(markup).toContain('class="hidden font-mono leading-none"');
		});
	}

	test("the editable editor mounts them", async () => {
		const view = await mountEditor(
			createPlateEditor({
				plugins: KIT,
				value: [...equations(42), ...equations(undefined)],
			}),
		);
		const rendered = view.host.querySelectorAll(".katex").length;
		const text = view.host.textContent ?? "";
		await view.unmount();

		expect(rendered).toBeGreaterThan(0);
		expect(text).toContain("42");
	});
});

const DANGEROUS_NODE_TYPES = new Set(["iframe", "object", "script", "style"]);
const DANGEROUS_URL =
	/^\s*(?:j\s*a\s*v\s*a\s*s\s*c\s*r\s*i\s*p\s*t|vbscript):|^\s*data:(?!image\/)/i;

/** Node props that would run script once rendered or exported. */
function findUnsafeNodes(nodes: unknown): string[] {
	const hits = new Set<string>();
	const visit = (node: unknown) => {
		if (Array.isArray(node)) return node.forEach(visit);
		if (!node || typeof node !== "object") return;
		const record = node as Record<string, unknown>;
		const type = typeof record.type === "string" ? record.type : "text";
		if (DANGEROUS_NODE_TYPES.has(type)) hits.add(`type:${type}`);
		for (const [key, value] of Object.entries(record)) {
			if (/^on[a-z]/i.test(key)) hits.add(`${type}.${key}`);
			if (typeof value === "string" && DANGEROUS_URL.test(value))
				hits.add(`${type}.${key}`);
		}
		if (Array.isArray(record.children)) record.children.forEach(visit);
	};
	visit(nodes);
	return [...hits].sort();
}

const HOSTILE_IMPORTS = [
	...HTML_IMPORT_FIXTURES,
	{
		id: "script-element",
		html: "<p>a</p><script>window.__xss=11</script><style>p{}</style><p>b</p>",
	},
	{
		id: "img-javascript-src",
		html: '<img src="javascript:window.__xss=12"><p>x</p>',
	},
	{
		id: "tab-obfuscated-href",
		html: '<p><a href="java\tscript:window.__xss=13">t</a></p>',
	},
	{
		id: "iframe-data-html",
		html: '<iframe src="data:text/html,<script>window.__xss=14</script>"></iframe><p>d</p>',
	},
	{ id: "vbscript-href", html: '<p><a href="vbscript:msgbox(1)">v</a></p>' },
];

describe("Import from HTML", () => {
	const importHtml = (html: string) =>
		quietly(() =>
			deserializeHtmlFile(createPlateEditor({ plugins: KIT }), html),
		);

	for (const fixture of HOSTILE_IMPORTS) {
		test(`${fixture.id}: parsed off the live document, nothing executable survives`, async () => {
			liveMarkupWrites.length = 0;
			const bodyBefore = window.document.body.innerHTML;
			const nodes = await importHtml(fixture.html);

			expect(liveMarkupWrites).toEqual([]);
			expect(window.document.body.innerHTML).toBe(bodyBefore);
			expect(findUnsafeNodes(nodes)).toEqual([]);
			expect(JSON.stringify(nodes)).not.toContain("__xss");
		});
	}

	test("markup that is not a Plate export imports", async () => {
		expect(outline(await importHtml("<h1>Title</h1><p>Body</p>"))).toBe(
			'h1<"Title"> | p<"Body">',
		);
		expect(
			outline(
				await importHtml(
					"<!doctype html><html><head><title>T</title><style>p{}</style></head><body><h2>Hi</h2><p>there</p></body></html>",
				),
			),
		).toBe('h2<"Hi"> | p<"there">');
	});

	test("a Plate export imports from its editor root", async () => {
		const html =
			HTML_IMPORT_FIXTURES.find((fixture) => fixture.id === "H09-plate-export")
				?.html ?? "";
		expect(outline(await importHtml(html))).toBe(
			'p<"exported"{bold}> | img(x)<"">',
		);
	});

	test("links, embeds and images with safe URLs are kept", async () => {
		expect(
			outline(
				await importHtml(
					'<p><a href="https://x.io">l</a></p><iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ"></iframe><img src="data:image/png;base64,AAAA">',
				),
			),
		).toBe(
			'p<a(https://x.io)<"l">> | media_embed(https://www.youtube.com/embed/dQw4w9WgXcQ)<""> | img(data:image/png;base64,AAAA)<"">',
		);
	});

	test("H07: an iframe with a javascript: src is dropped, its sibling kept", async () => {
		const html =
			HTML_IMPORT_FIXTURES.find((fixture) => fixture.id === "H07-iframe")
				?.html ?? "";
		expect(outline(await importHtml(html))).toBe('p<"c">');
	});
});

/** Every `url` in the tree, in document order. */
function urlsOf(nodes: unknown): string[] {
	const urls: string[] = [];
	const visit = (node: unknown) => {
		if (Array.isArray(node)) return node.forEach(visit);
		if (!node || typeof node !== "object") return;
		const record = node as Record<string, unknown>;
		if (typeof record.url === "string") urls.push(record.url);
		if (Array.isArray(record.children)) record.children.forEach(visit);
	};
	visit(nodes);
	return urls;
}

const unsafeUrlsOf = (nodes: unknown) =>
	urlsOf(nodes).filter((url) => DANGEROUS_URL.test(url));

/** A pasted `<script>` body is inert paragraph text; only props could carry a payload. */
const withoutText = (key: string, value: unknown) =>
	key === "text" ? undefined : value;

const xssFlags = () => [
	(window as unknown as Record<string, unknown>).__xss,
	(globalThis as Record<string, unknown>).__xss,
];

const SAFE_URLS = [
	"https://x.io/",
	"mailto:a@b.co",
	"/docs/page",
	"#section",
	"storage://docs/a.png",
	"blob:http://localhost:3000/5f0c",
	"data:image/png;base64,AAAA",
] as const;

const SCRIPT_URLS = [
	"javascript:window.__xss=21",
	" JaVaScRiPt:window.__xss=22",
	"java\tscript:window.__xss=23",
	"vbscript:msgbox(1)",
	"data:text/html,<script>window.__xss=24</script>",
] as const;

describe("Paste from HTML", () => {
	const pasteHtml = (html: string) =>
		quietly(() => {
			const editor = createEditor(paragraph());
			editor.tf.insertData(clipboard({ "text/html": html, "text/plain": "" }));
			return editor.children;
		});

	for (const fixture of HOSTILE_IMPORTS) {
		test(`${fixture.id}: nothing executable survives, nothing runs`, async () => {
			liveMarkupWrites.length = 0;
			const bodyBefore = window.document.body.innerHTML;
			const nodes = await pasteHtml(fixture.html);
			await new Promise((resolve) => setTimeout(resolve, 10));

			expect(unsafeUrlsOf(nodes)).toEqual([]);
			expect(findUnsafeNodes(nodes)).toEqual([]);
			expect(JSON.stringify(nodes, withoutText)).not.toContain("__xss");
			expect(liveMarkupWrites).toEqual([]);
			expect(window.document.body.innerHTML).toBe(bodyBefore);
			expect(xssFlags()).toEqual([undefined, undefined]);
		});
	}

	test("H07: the iframe is dropped, its sibling kept", async () => {
		const html =
			HTML_IMPORT_FIXTURES.find((fixture) => fixture.id === "H07-iframe")
				?.html ?? "";
		expect(outline(await pasteHtml(html))).toBe('p<"c">');
	});

	test("H02: a javascript: link keeps its text", async () => {
		const html =
			HTML_IMPORT_FIXTURES.find((fixture) => fixture.id === "H02-js-href")
				?.html ?? "";
		expect(outline(await pasteHtml(html))).toBe('p<"link">');
	});

	test("links, embeds and images with safe URLs are kept", async () => {
		const nodes = await pasteHtml(
			'<p><a href="https://x.io/">l</a> <a href="mailto:a@b.co">m</a> <a href="/docs/page">r</a></p><iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ"></iframe><img src="data:image/png;base64,AAAA"><p>end</p>',
		);
		expect(urlsOf(nodes)).toEqual([
			"https://x.io/",
			"mailto:a@b.co",
			"/docs/page",
			"https://www.youtube.com/embed/dQw4w9WgXcQ",
			"data:image/png;base64,AAAA",
		]);
	});
});

describe("Markdown never yields script URLs", () => {
	const hostile = MARKDOWN_FIXTURES.filter((fixture) => fixture.malicious);
	const editable = () => createPlateEditor({ plugins: KIT });
	const readOnly = () => getStaticParseWorker(BaseEditorKit);

	for (const fixture of hostile) {
		test(`${fixture.id}: parser, read-only and editable seeds`, async () => {
			const parsed = await quietly(() => [
				editable().api.markdown.deserialize(fixture.markdown),
				readOnly().api.markdown.deserialize(fixture.markdown),
				safeDeserialize(
					readOnly(),
					fixture.markdown,
					true,
					RICH_REMARK_PLUGINS,
				),
				safeDeserialize(
					editable(),
					fixture.markdown,
					true,
					RICH_REMARK_PLUGINS,
				),
			]);
			for (const nodes of parsed) expect(unsafeUrlsOf(nodes)).toEqual([]);
		});

		test(`${fixture.id}: paste as text`, async () => {
			const nodes = await quietly(() => {
				const editor = createEditor(paragraph());
				editor.tf.insertData(clipboard({ "text/plain": fixture.markdown }));
				return editor.children;
			});
			expect(unsafeUrlsOf(nodes)).toEqual([]);
		});
	}

	test("script links become their text, script images are dropped", () => {
		const editor = editable();
		expect(
			outline(
				editor.api.markdown.deserialize(
					"a [plain](javascript:x) b\n\n![x](javascript:y)\n\n[vb](vbscript:z) and [ok](https://x.io/)",
				),
			),
		).toBe('p<"a plain b"> | p<"vb and " a(https://x.io/)<"ok">>');
	});

	test("links and images with safe URLs are kept", () => {
		const markdown = [
			"[a](https://x.io/) [m](mailto:a@b.co) [r](/docs/page) [h](#section)",
			"![s](storage://docs/a.png)",
			"![d](data:image/png;base64,AAAA)",
		].join("\n\n");
		for (const editor of [editable(), readOnly()])
			expect(urlsOf(editor.api.markdown.deserialize(markdown))).toEqual([
				"https://x.io/",
				"mailto:a@b.co",
				"/docs/page",
				"#section",
				"storage://docs/a.png",
				"data:image/png;base64,AAAA",
			]);
	});
});

describe("Inserted links and media with script URLs", () => {
	const MEDIA_TYPES = ["img", "video", "audio", "file", "media_embed"] as const;
	const withText = () => createEditor(paragraph("before"));

	for (const type of MEDIA_TYPES) {
		test(`${type}: a script URL is removed, safe ones stay`, async () => {
			const editor = withText();
			await quietly(() => {
				for (const url of [...SCRIPT_URLS, ...SAFE_URLS])
					editor.tf.insertNodes(
						{ type, url, children: [{ text: "" }] },
						{ at: [editor.children.length] },
					);
			});
			expect(urlsOf(editor.children)).toEqual([...SAFE_URLS]);
			expect(outline(editor.children.slice(0, 1))).toBe('p<"before">');
		});
	}

	test("a link with a script URL is unwrapped to its text", () => {
		const editor = withText();
		for (const url of SCRIPT_URLS)
			editor.tf.insertNodes(
				{ type: "a", url, children: [{ text: "x" }] },
				{ select: true },
			);
		expect(outline(editor.children)).toBe('p<"beforexxxxx">');
	});

	test("links with safe URLs stay links", () => {
		const editor = withText();
		for (const url of SAFE_URLS)
			editor.tf.insertNodes(
				{ type: "a", url, children: [{ text: "x" }] },
				{ select: true },
			);
		expect(urlsOf(editor.children)).toEqual([...SAFE_URLS]);
	});

	test("pointing an existing link at a script URL unwraps it", () => {
		const editor = createEditor([
			{
				type: "p",
				children: [
					{ text: "see " },
					{ type: "a", url: "https://x.io/", children: [{ text: "here" }] },
					{ text: "" },
				],
			},
		]);
		editor.tf.setNodes({ url: "javascript:window.__xss=25" }, { at: [0, 1] });
		expect(outline(editor.children)).toBe('p<"see here">');
	});

	test("a pasted Plate fragment is filtered too", () => {
		const editor = withText();
		editor.tf.insertFragment([
			{
				type: "p",
				children: [
					{ text: "a " },
					{ type: "a", url: "javascript:x", children: [{ text: "js" }] },
					{ text: " " },
					{ type: "a", url: "https://x.io/", children: [{ text: "ok" }] },
					{ text: "" },
				],
			},
			{ type: "video", url: "vbscript:y", children: [{ text: "" }] },
			{ type: "p", children: [{ text: "tail" }] },
		]);
		expect(outline(editor.children)).toBe(
			'p<"beforea js " a(https://x.io/)<"ok"> ""> | p<"tail">',
		);
	});

	test("pasting nothing but an unsafe embed leaves an editable paragraph", async () => {
		const editor = createEditor(paragraph());
		await quietly(() =>
			editor.tf.insertData(
				clipboard({
					"text/html": '<iframe src="javascript:window.__xss=26"></iframe>',
					"text/plain": "",
				}),
			),
		);
		expect(outline(editor.children)).toBe('p<"">');
		editor.tf.select(editor.api.end([]));
		editor.tf.insertText("x");
		expect(outline(editor.children)).toBe('p<"x">');
	});

	test("stored documents load untouched until edited", () => {
		const stored = STORED_FIXTURES.find(
			(fixture) => fixture.id === "S09-malicious-stored",
		);
		if (!stored) throw new Error("the corpus has no S09");
		const editor = createPlateEditor({
			plugins: KIT,
			value: structuredClone(stored.nodes) as Value,
		});
		expect(urlsOf(editor.children)).toEqual(urlsOf(stored.nodes));
	});
});
