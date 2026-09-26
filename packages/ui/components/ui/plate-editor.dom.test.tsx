import {
	afterAll,
	beforeAll,
	describe,
	expect,
	setDefaultTimeout,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import type { PlateEditor } from "platejs/react";
import { type ReactElement, act, createElement, useEffect } from "react";
import { plainTextFromRichContent } from "../../lib/plate-text";
import {
	HTML_IMPORT_FIXTURES,
	MARKDOWN_FIXTURES,
	STORED_FIXTURES,
	STREAMING_PREFIX_FIXTURES,
	toEnvelope,
} from "../editor/__fixtures__/plate-corpus";
import {
	LEGACY_EDITABLE_ENVELOPES,
	LEGACY_STATIC_ENVELOPES,
	PRODUCTION_ENVELOPES,
} from "../editor/__fixtures__/plate-legacy-49";
import {
	collectNodeIds,
	countElements,
	findUnsafeMarkup,
	fingerprint,
	formatHtml,
	normalizeNodeIds,
} from "../editor/__fixtures__/plate-snapshot";

// DateElementStatic formats in local time.
process.env.TZ = "UTC";

// A loaded machine pushes the heavier renders past bun's 5 s default.
setDefaultTimeout(60_000);

// Plate skips NodeIdPlugin under NODE_ENV=test. The app stamps node ids, and
// that stamping is what makes the editable editor emit once on mount.
const previousNodeEnv = process.env.NODE_ENV;
Object.assign(process.env, { NODE_ENV: "development" });
afterAll(() => {
	Object.assign(process.env, { NODE_ENV: previousNodeEnv });
});

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

/** Everything Plate, Radix, dnd-kit, KaTeX/DOMPurify and the chart fences touch. */
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

/**
 * Installed before react-dom loads (it probes the DOM once at import) and again
 * in `beforeAll`, because other test files assign their own window on load.
 */
function installDomGlobals() {
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
	Object.assign(globalThis, globals);
}
installDomGlobals();
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

/**
 * Markup handed to `innerHTML` of an element owned by the live document. Event
 * handlers in such markup run in a browser (the CVE class the Plate upgrade
 * fixes); a DOMParser document is inert.
 */
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
const handlerBearingWrites = () =>
	liveMarkupWrites.filter((markup) => /\son[a-z]+\s*=/i.test(markup));

const { createRoot } = await import("react-dom/client");
const { KEYS, NodeApi } = await import("platejs");
const { getEditorDOMFromHtmlString } = await import("platejs/static");
const { PlateController, createPlateEditor, useEditorMounted, useEditorRef } =
	await import("platejs/react");
const { createEditorKit } = await import("../editor/editor-kit");
const { insertBlock, setBlockType } = await import("../editor/transforms");
const { StreamingTextEditor } = await import("./streaming-text-editor");
const { PLATE_JSON_PREFIX, TextEditor } = await import("./text-editor");

beforeAll(installDomGlobals);

/** Parse fallbacks, lowlight, KaTeX quirks mode and demo discussions log noise. */
async function quietly<T>(run: () => Promise<T>): Promise<T> {
	const { error, warn, log } = console;
	const mute = () => {};
	Object.assign(console, { error: mute, warn: mute, log: mute });
	try {
		return await run();
	} finally {
		Object.assign(console, { error, warn, log });
	}
}

/** Lazy directive/embed renderers and the lazy editable editor need a few ticks. */
async function settle(rounds = 6) {
	for (let round = 0; round < rounds; round++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 25));
		});
	}
}

async function mount(element: ReactElement) {
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	const container = host as unknown as HTMLElement;
	const root = createRoot(container);
	await quietly(async () => {
		await act(async () => {
			root.render(element);
		});
		await settle();
	});
	return {
		container,
		unmount: () =>
			quietly(async () => {
				await act(async () => root.unmount());
				host.remove();
			}),
	};
}

const parseEnvelope = (content: string): unknown[] =>
	JSON.parse(content.slice(PLATE_JSON_PREFIX.length));

const markdownFixture = (id: string) => {
	const fixture = MARKDOWN_FIXTURES.find((candidate) => candidate.id === id);
	if (!fixture) throw new Error(`no markdown fixture ${id}`);
	return fixture.markdown;
};

const hasClass =
	(name: string) => (_tag: string, attributes: Map<string, string>) =>
		(attributes.get("class") ?? "").split(/\s+/).includes(name);

type KatexCase = {
	readonly id: string;
	readonly content: string;
	readonly isMarkdown: boolean;
	readonly streaming?: boolean;
};

const KATEX_CASES: readonly KatexCase[] = [
	...MARKDOWN_FIXTURES.filter((fixture) => fixture.usesKatex).map(
		(fixture) => ({
			id: fixture.id,
			content: fixture.markdown,
			isMarkdown: true,
		}),
	),
	...STORED_FIXTURES.filter((fixture) => fixture.usesKatex).map((fixture) => ({
		id: fixture.id,
		content: toEnvelope(fixture.nodes),
		isMarkdown: false,
	})),
	...STREAMING_PREFIX_FIXTURES.filter((fixture) => fixture.usesKatex).map(
		(fixture) => ({
			id: fixture.id,
			content: fixture.content,
			isMarkdown: true,
			streaming: true,
		}),
	),
];

describe("KaTeX renders with a DOM", () => {
	for (const fixture of KATEX_CASES) {
		test(fixture.id, async () => {
			const view = await mount(
				fixture.streaming
					? createElement(StreamingTextEditor, { content: fixture.content })
					: createElement(TextEditor, {
							initialContent: fixture.content,
							isMarkdown: fixture.isMarkdown,
						}),
			);
			const markup = view.container.innerHTML;
			await view.unmount();

			expect(formatHtml(markup)).toMatchSnapshot();
			expect(findUnsafeMarkup(markup)).toEqual([]);
			if (!fixture.streaming)
				expect(countElements(markup, hasClass("katex"))).toBeGreaterThan(0);
		});
	}

	test("the finished M11 stream equals the settled render", async () => {
		const markdown = markdownFixture("M11-math");
		const streamed = await mount(
			createElement(StreamingTextEditor, { content: markdown }),
		);
		const streamedMarkup = streamed.container.innerHTML;
		await streamed.unmount();
		const settled = await mount(
			createElement(TextEditor, { initialContent: markdown, isMarkdown: true }),
		);
		const settledMarkup = settled.container.innerHTML;
		await settled.unmount();

		expect(formatHtml(streamedMarkup)).toBe(formatHtml(settledMarkup));
	});

	test("dollar amounts in prose stay text", async () => {
		const view = await mount(
			createElement(TextEditor, {
				initialContent: markdownFixture("M11-math"),
				isMarkdown: true,
			}),
		);
		const paragraph = Array.from(
			view.container.querySelectorAll('[data-slate-type="p"]'),
		).find((element) => element.textContent?.includes("It costs"));
		await view.unmount();

		expect(paragraph?.textContent).toBe("It costs $5 today and $10 tomorrow.");
		expect(paragraph?.querySelector(".katex")).toBeNull();
	});

	for (const envelope of [
		...LEGACY_STATIC_ENVELOPES,
		...LEGACY_EDITABLE_ENVELOPES,
	].filter((candidate) => candidate.usesKatex)) {
		test(`${envelope.id} keeps its text and structure`, async () => {
			const view = await mount(
				createElement(TextEditor, {
					initialContent: envelope.content,
					isMarkdown: false,
				}),
			);
			const markup = view.container.innerHTML;
			await view.unmount();

			expect(fingerprint({ html: markup })).toMatchSnapshot();
			expect(findUnsafeMarkup(markup)).toEqual([]);
		});
	}
});

describe("read-only chips", () => {
	test("focus-node and user-mention chips report clicks", async () => {
		const focused: string[] = [];
		const mentioned: string[] = [];
		const view = await mount(
			createElement(TextEditor, {
				initialContent: markdownFixture("M04-special-links"),
				isMarkdown: true,
				onFocusNode: (nodeId: string) => focused.push(nodeId),
				onUserMention: (sub: string) => mentioned.push(sub),
			}),
		);
		const click = async (selector: string) => {
			const element = view.container.querySelector(selector) as HTMLElement;
			expect(element).not.toBeNull();
			await act(async () => element.click());
		};
		await click('[data-focus-node-id="node_abc123"]');
		await click('[data-user-mention-sub="sub-alice"]');
		await view.unmount();

		expect(focused).toEqual(["node_abc123"]);
		expect(mentioned).toEqual(["sub-alice"]);
	});
});

const EDITOR_ID = "rendered-editor";

function EditorProbe({
	onReady,
}: Readonly<{ onReady: (editor: PlateEditor) => void }>) {
	const editor = useEditorRef(EDITOR_ID);
	const mounted = useEditorMounted(EDITOR_ID);
	useEffect(() => {
		if (mounted && !editor.meta.isFallback) onReady(editor);
	}, [editor, mounted, onReady]);
	return null;
}

/** Plate stamps these with random node ids on every mount. */
const EDITABLE_VOLATILE_ATTRIBUTES = ["data-block-id", "id"];

/** Mounts the app's editable `TextEditor` (lazy `TextEditorEditable`, full `createEditorKit`). */
async function mountEditable(initialContent: string, isMarkdown: boolean) {
	const changes: string[] = [];
	let editor: PlateEditor | undefined;
	const view = await mount(
		createElement(
			PlateController,
			null,
			createElement(TextEditor, {
				initialContent,
				isMarkdown,
				editable: true,
				onChange: (content: string) => changes.push(content),
			}),
			createElement(EditorProbe, {
				onReady: (ready: PlateEditor) => {
					editor = ready;
				},
			}),
		),
	);
	if (!editor) throw new Error("the editable editor never mounted");
	const editable = view.container.querySelector('[data-slate-editor="true"]');
	if (!editable) throw new Error("no contenteditable root");
	return {
		...view,
		editor,
		changes,
		editableMarkup: () =>
			formatHtml(editable.outerHTML, {
				volatileAttributes: EDITABLE_VOLATILE_ATTRIBUTES,
			}),
	};
}

const EDITABLE_DOCUMENTS: ReadonlyArray<{
	readonly id: string;
	readonly content: string;
	readonly isMarkdown: boolean;
}> = [
	{
		id: "M26-mixed-report (chat edit dialog)",
		content: markdownFixture("M26-mixed-report"),
		isMarkdown: true,
	},
	{
		id: "M07-lists (markdown seed)",
		content: markdownFixture("M07-lists"),
		isMarkdown: true,
	},
	{
		id: "L49-editable-M14-mdx-blocks (comment edit dialog)",
		content:
			LEGACY_EDITABLE_ENVELOPES.find(
				(envelope) => envelope.id === "L49-editable-M14-mdx-blocks",
			)?.content ?? "",
		isMarkdown: true,
	},
	{
		id: "S05-marks-and-styles (RichText authoring)",
		content: toEnvelope(
			STORED_FIXTURES.find((fixture) => fixture.id === "S05-marks-and-styles")
				?.nodes ?? [],
		),
		isMarkdown: false,
	},
	{
		id: `${PRODUCTION_ENVELOPES[0].id} (board comment)`,
		content: PRODUCTION_ENVELOPES[0].content,
		isMarkdown: true,
	},
];

describe("editable editor", () => {
	test("renders the fixed toolbar", async () => {
		const view = await mountEditable("Toolbar probe", true);
		const toolbar = view.container.querySelector('[role="toolbar"]');
		const markup = toolbar ? formatHtml(toolbar.outerHTML) : "no toolbar";
		await view.unmount();

		expect(markup).toMatchSnapshot();
	});

	for (const sample of EDITABLE_DOCUMENTS) {
		test(`${sample.id} mounts and normalizes without losing content`, async () => {
			const view = await mountEditable(sample.content, sample.isMarkdown);
			const markup = view.editableMarkup();
			const { changes } = view;
			const children = structuredClone(view.editor.children);
			await view.unmount();

			const input = sample.content.startsWith(PLATE_JSON_PREFIX)
				? parseEnvelope(sample.content)
				: [];
			const inputIds = collectNodeIds(input);
			const persisted = `${PLATE_JSON_PREFIX}${JSON.stringify(children)}`;

			expect(markup).toMatchSnapshot("editable DOM");
			expect({
				emittedOnMount: changes.length,
				document: normalizeNodeIds(children, inputIds),
			}).toMatchSnapshot("document after mount");
			expect(changes.length).toBeLessThanOrEqual(1);
			if (changes.length === 1) expect(changes[0]).toBe(persisted);
			for (const id of inputIds) expect(collectNodeIds(children)).toContain(id);
			if (input.length > 0) {
				expect(plainTextFromRichContent(persisted)).toBe(
					plainTextFromRichContent(sample.content),
				);
			}
		});
	}

	test.failing("keeps an ordered list's start number", async () => {
		const view = await mountEditable("3. three\n4. four", true);
		const [first] = view.editor.children;
		await view.unmount();

		expect(first).toMatchObject({ listStyleType: "decimal", listStart: 3 });
	});

	test("typing, marks, list and table transforms and undo edit the document", async () => {
		const view = await mountEditable(
			"First paragraph\n\nSecond paragraph",
			true,
		);
		const { editor, changes } = view;
		const current = () => normalizeNodeIds(structuredClone(editor.children));
		const editable = view.container.querySelector('[data-slate-editor="true"]');
		// Selection lands in its own tick, as it does for real input: in the same
		// tick Slate history merges the edit into the previous undo batch.
		const run = (transform: () => void) =>
			quietly(async () => {
				await act(async () => transform());
				await settle(2);
			});

		await run(() => editor.tf.select(editor.api.end([0])));
		await run(() => editor.tf.insertText(" typed"));
		expect(NodeApi.string(editor.children[0])).toBe("First paragraph typed");

		await run(() =>
			editor.tf.select({
				anchor: { path: [0, 0], offset: 16 },
				focus: { path: [0, 0], offset: 21 },
			}),
		);
		await run(() => editor.tf.toggleMark(KEYS.bold));
		expect(editor.children[0].children).toEqual([
			{ text: "First paragraph " },
			{ text: "typed", bold: true },
		]);
		expect(editable?.querySelector("strong")?.textContent).toBe("typed");
		const marked = current();

		await run(() => editor.tf.select(editor.api.start([1])));
		await run(() => setBlockType(editor, KEYS.ul));
		expect(editor.children[1]).toMatchObject({
			type: "p",
			indent: 1,
			listStyleType: "disc",
		});
		expect(editable?.querySelector("ul li")?.textContent).toContain(
			"Second paragraph",
		);
		const listed = current();

		await run(() => editor.tf.select(editor.api.end([1])));
		await run(() => insertBlock(editor, KEYS.table));
		expect(editor.children.map((node) => node.type)).toContain(KEYS.table);
		expect(editable?.querySelectorAll("table").length).toBeGreaterThan(0);
		const tabled = current();

		await run(() => editor.tf.undo());
		expect(current()).toEqual(listed);
		expect(editable?.querySelectorAll("table")).toHaveLength(0);
		await run(() => editor.tf.undo());
		expect(current()).toEqual(marked);
		await run(() => editor.tf.redo());
		await run(() => editor.tf.redo());
		expect(current()).toEqual(tabled);

		const lastChange = changes[changes.length - 1];
		const markup = view.editableMarkup();
		await view.unmount();

		expect(lastChange).toBe(
			`${PLATE_JSON_PREFIX}${JSON.stringify(editor.children)}`,
		);
		expect(tabled).toMatchSnapshot("document after the table insert");
		expect(markup).toMatchSnapshot("editable DOM after the edits");
	});
});

const DANGEROUS_NODE_TYPES = new Set(["iframe", "object", "script", "style"]);
const DANGEROUS_URL = /^\s*(?:javascript|vbscript):|^\s*data:text\/html/i;

/** Node props that would execute once rendered or exported. */
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

/** Pinned Plate 49 behaviour: an iframe's javascript: src survives as a media_embed url. */
const KNOWN_UNSAFE_IMPORTS: Readonly<Record<string, readonly string[]>> = {
	"H07-iframe": ["media_embed.url"],
};

function pasteHtml(html: string) {
	const editor = createPlateEditor({
		plugins: createEditorKit(),
		value: [{ type: "p", children: [{ text: "" }] }],
	});
	editor.tf.select({ path: [0, 0], offset: 0 });
	const data: Record<string, string> = { "text/html": html, "text/plain": "" };
	editor.tf.insertData({
		getData: (type: string) => data[type] ?? "",
		setData: () => {},
		types: Object.keys(data),
		files: [],
		items: [],
	} as unknown as DataTransfer);
	return normalizeNodeIds(editor.children);
}

function deserializeHtmlString(html: string) {
	const editor = createPlateEditor({ plugins: createEditorKit() });
	return editor.api.html.deserialize({ element: html });
}

describe("HTML deserialization", () => {
	for (const fixture of HTML_IMPORT_FIXTURES) {
		describe(fixture.id, () => {
			test("paste yields inert nodes without touching the live document", async () => {
				liveMarkupWrites.length = 0;
				const nodes = await quietly(async () => pasteHtml(fixture.html));

				expect(nodes).toMatchSnapshot();
				expect(findUnsafeNodes(nodes)).toEqual([
					...(KNOWN_UNSAFE_IMPORTS[fixture.id] ?? []),
				]);
				expect(handlerBearingWrites()).toEqual([]);
			});

			test("the html api yields inert nodes", async () => {
				const nodes = await quietly(async () =>
					deserializeHtmlString(fixture.html),
				);

				expect(normalizeNodeIds(nodes)).toMatchSnapshot();
				expect(findUnsafeNodes(nodes)).toEqual([
					...(KNOWN_UNSAFE_IMPORTS[fixture.id] ?? []),
				]);
			});
		});
	}

	test("H10 paste keeps headings, marks, lists, tables, quotes and code", async () => {
		const nodes = await quietly(async () =>
			pasteHtml(
				HTML_IMPORT_FIXTURES.find((fixture) => fixture.id === "H10-benign")
					?.html ?? "",
			),
		);
		const types = new Set<string>();
		const marks = new Set<string>();
		const visit = (node: Record<string, unknown>) => {
			if (typeof node.text === "string") {
				for (const key of Object.keys(node)) if (key !== "text") marks.add(key);
				return;
			}
			types.add(String(node.type));
			if (typeof node.listStyleType === "string") types.add("list");
			(node.children as Record<string, unknown>[]).forEach(visit);
		};
		(nodes as Record<string, unknown>[]).forEach(visit);

		for (const type of ["h1", "p", "list", "table", "blockquote", "code_block"])
			expect(types).toContain(type);
		for (const mark of ["bold", "italic", "underline", "strikethrough", "code"])
			expect(marks).toContain(mark);
	});

	test.failing(
		"an iframe with a javascript: src never becomes a media embed",
		async () => {
			const nodes = await quietly(async () =>
				pasteHtml(
					HTML_IMPORT_FIXTURES.find((fixture) => fixture.id === "H07-iframe")
						?.html ?? "",
				),
			);
			expect(findUnsafeNodes(nodes)).toEqual([]);
		},
	);

	test.failing(
		"the html api never parses markup into the live document",
		async () => {
			liveMarkupWrites.length = 0;
			await quietly(async () => {
				for (const fixture of HTML_IMPORT_FIXTURES)
					deserializeHtmlString(fixture.html);
			});
			expect(handlerBearingWrites()).toEqual([]);
		},
	);

	test.failing(
		"Import from HTML accepts markup that is not a Plate export",
		async () => {
			const editor = createPlateEditor({ plugins: createEditorKit() });
			const nodes = await quietly(async () =>
				editor.api.html.deserialize({
					element: getEditorDOMFromHtmlString(
						"<h1>Title</h1><p>Body</p>",
					) as HTMLElement,
				}),
			);
			expect(nodes.map((node) => node.type)).toEqual(["h1", "p"]);
		},
	);
});
