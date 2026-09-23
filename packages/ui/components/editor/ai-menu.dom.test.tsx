import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	setDefaultTimeout,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import type { Value } from "platejs";
import { act } from "react";
import type { IResponse } from "../../lib";
import { type IHistoryMessage, IRole } from "../../lib/schema/llm/history";
import type { IResponseChunk } from "../../lib/schema/llm/response-chunk";
import type { IAIState } from "../../state/backend-state/ai-state";
import type { EditorChat } from "./use-chat";

setDefaultTimeout(60_000);

// Plate skips NodeIdPlugin under NODE_ENV=test; the menu block-selects by node id.
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
	"HTMLInputElement",
	"HTMLTextAreaElement",
	"HTMLButtonElement",
	"HTMLIFrameElement",
	"HTMLTemplateElement",
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
	"innerWidth",
	"innerHeight",
	"scrollX",
	"scrollY",
	"pageXOffset",
	"pageYOffset",
	"visualViewport",
] as const;

/** Before react-dom loads (it probes the DOM at import) and again per file run. */
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

// SDK stream callbacks and the menu's anchor timer land between act() scopes.
const consoleError = console.error;
console.error = (...args: unknown[]) => {
	if (String(args[0]).includes("not wrapped in act(")) return;
	consoleError(...args);
};
afterAll(() => {
	console.error = consoleError;
});

const { createRoot } = await import("react-dom/client");
const { KEYS, NodeApi } = await import("platejs");
const { Plate, createPlateEditor } = await import("platejs/react");
const { AIChatPlugin, triggerCopilotSuggestion } = await import(
	"@platejs/ai/react"
);
const { useBackendStore } = await import("../../state/backend-state");
const { PROMPT_TEMPLATES } = await import("./ai-prompt");
const { AIUsageAppContext } = await import("./ai-usage-context");
const { createEditorKit } = await import("./editor-kit");
const { Editor, EditorContainer } = await import("./ui/editor");
const { deserializeMarkdownFile } = await import("./ui/import-toolbar-button");
const { CODE_MARKUP_FIXTURES } = await import("./__fixtures__/plate-corpus");

beforeAll(installDomGlobals);

type Request = {
	messages: IHistoryMessage[];
	appId?: string;
	push: (...deltas: string[]) => void;
	finish: (reason?: string) => void;
	cancelled: () => boolean;
};

/** An `IAIState` whose response streams the test drives delta by delta. */
const streamingBackend = () => {
	const requests: Request[] = [];
	const aiState: IAIState = {
		streamChatComplete: async (messages, appId) => {
			let controller:
				| ReadableStreamDefaultController<IResponseChunk[]>
				| undefined;
			let cancelled = false;
			const stream = new ReadableStream<IResponseChunk[]>({
				start(streamController) {
					controller = streamController;
				},
				cancel() {
					cancelled = true;
				},
			});
			requests.push({
				messages,
				appId,
				push: (...deltas) => {
					if (cancelled) return;
					controller?.enqueue(
						deltas.map((content) => ({
							id: "chunk",
							choices: [{ index: 0, delta: { content } }],
						})),
					);
				},
				finish: (reason = "stop") => {
					if (cancelled) return;
					controller?.enqueue([
						{ id: "chunk", choices: [{ index: 0, finish_reason: reason }] },
					]);
					controller?.close();
				},
				cancelled: () => cancelled,
			});
			return stream;
		},
		chatComplete: async () => ({ choices: [] }) as unknown as IResponse,
	};
	return { aiState, requests };
};

type Backend = ReturnType<typeof streamingBackend>;

const useFakeBackend = (backend: Backend) =>
	useBackendStore.getState().setBackend({ aiState: backend.aiState } as never);

const mounted = new Set<() => Promise<void>>();

afterEach(async () => {
	for (const unmount of mounted) await unmount();
	useBackendStore.setState({ backend: null });
});

/** Streams, lazy renderers and Radix portals need a few macrotasks to land. */
async function settle(rounds = 4) {
	for (let round = 0; round < rounds; round++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 20));
		});
	}
}

const DOCUMENT: Value = [
	{ type: "p", children: [{ text: "Hello there." }] },
	{ type: "p", children: [{ text: "Tail." }] },
];

type EditorHandle = Awaited<ReturnType<typeof mountEditor>>;

async function mountEditor({
	appId,
	value = DOCUMENT,
}: { appId?: string; value?: Value } = {}) {
	const editor = createPlateEditor({
		plugins: createEditorKit(appId),
		value: structuredClone(value),
	});
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	const root = createRoot(host as unknown as HTMLElement);
	const render = async (scope?: string) => {
		await act(async () => {
			root.render(
				<AIUsageAppContext.Provider value={scope}>
					<Plate editor={editor}>
						<EditorContainer>
							<Editor variant="none" />
						</EditorContainer>
					</Plate>
				</AIUsageAppContext.Provider>,
			);
		});
		await settle();
	};
	const unmount = async () => {
		if (!mounted.delete(unmount)) return;
		await act(async () => root.unmount());
		host.remove();
	};
	mounted.add(unmount);
	await render(appId);
	return { editor, host, render, unmount };
}

const chatOf = ({ editor }: EditorHandle) =>
	editor.getOption(AIChatPlugin, "chat") as EditorChat & {
		isLoading?: boolean;
	};

const blockTexts = ({ editor }: EditorHandle) =>
	editor.children.map((node) => [node.type, NodeApi.string(node)]);

const hasAIMarks = ({ editor }: EditorHandle) =>
	editor.api.some({ at: [], match: (node) => !!node[KEYS.ai] });

const menuItems = () =>
	Array.from(window.document.body.querySelectorAll("[cmdk-item]"), (item) =>
		item.textContent?.trim(),
	);

async function run(action: () => unknown) {
	await act(async () => {
		action();
	});
	await settle();
}

async function clickMenuItem(label: string) {
	const item = Array.from(
		window.document.body.querySelectorAll("[cmdk-item]"),
	).find((candidate) => candidate.textContent?.trim() === label);
	if (!item) throw new Error(`no menu item "${label}" in ${menuItems()}`);
	await run(() => (item as unknown as HTMLElement).click());
}

async function stream(request: Request | undefined, ...deltas: string[]) {
	if (!request) throw new Error("the backend received no request");
	await run(() => request.push(...deltas));
}

const loadingBarText = ({ host }: EditorHandle) =>
	host.querySelector(".animate-spin")?.parentElement?.textContent ?? null;

async function openAtEndOf(handle: EditorHandle, block: number) {
	await run(() => {
		handle.editor.tf.select(handle.editor.api.end([block]));
		handle.editor.getApi(AIChatPlugin).aiChat.show();
	});
}

/** Selects "Hello" in the first block and opens the menu on it. */
async function selectAndOpen(handle: EditorHandle) {
	await run(() => {
		handle.editor.tf.select({
			anchor: { path: [0, 0], offset: 0 },
			focus: { path: [0, 0], offset: 5 },
		});
		handle.editor.getApi(AIChatPlugin).aiChat.show();
	});
}

type DomElement = NonNullable<ReturnType<typeof window.document.querySelector>>;

/** cmdk's input is controlled; React sees the native value setter plus an input event. */
async function type(input: DomElement, text: string) {
	await run(() => {
		const setter = Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set;
		setter?.call(input, text);
		input.dispatchEvent(new window.InputEvent("input", { bubbles: true }));
	});
}

async function pressEnter(input: DomElement) {
	await run(() =>
		input.dispatchEvent(
			new window.KeyboardEvent("keydown", {
				key: "Enter",
				code: "Enter",
				keyCode: 13,
				bubbles: true,
			}),
		),
	);
}

describe("editor AI menu over a streaming backend", () => {
	test("Continue writing streams below the cursor block and Accept keeps it", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor({ appId: "app-1" });

		await openAtEndOf(handle, 0);
		expect(menuItems()).toEqual([
			"Continue writing",
			"Add a summary",
			"Explain",
		]);

		await clickMenuItem("Continue writing");
		const [request] = backend.requests;
		expect(request.appId).toBe("app-1");
		expect(request.messages).toHaveLength(1);
		expect(request.messages[0].role).toBe(IRole.User);
		expect(request.messages[0].content).toBe(
			PROMPT_TEMPLATES.userDefault
				.replace("{block}", "Hello there.\n")
				.replace(
					"{prompt}",
					"Continue writing AFTER <Block> ONLY ONE SENTENCE. DONT REPEAT THE TEXT.",
				),
		);
		expect(chatOf(handle).status).toBe("submitted");
		expect(chatOf(handle).isLoading).toBe(true);
		expect(loadingBarText(handle)).toContain("Thinking...");
		expect(menuItems()).toEqual([]);

		await stream(request, "It was ", "a sunny");
		expect(handle.editor.getOption(AIChatPlugin, "streaming")).toBe(true);
		expect(loadingBarText(handle)).toContain("Writing...");
		expect(chatOf(handle).isLoading).toBe(true);
		expect(blockTexts(handle)).toEqual([
			["p", "Hello there."],
			["p", "It was a sunny"],
			[KEYS.aiChat, ""],
			["p", "Tail."],
		]);
		expect(hasAIMarks(handle)).toBe(true);

		await stream(request, " day.");
		await run(() => request.finish());
		expect(chatOf(handle).status).toBe("ready");
		expect(chatOf(handle).isLoading).toBe(false);
		expect(handle.editor.getOption(AIChatPlugin, "streaming")).toBe(false);
		expect(loadingBarText(handle)).toBeNull();
		expect(menuItems()).toEqual(["Accept", "Discard", "Try again"]);

		await clickMenuItem("Accept");
		expect(blockTexts(handle)).toEqual([
			["p", "Hello there."],
			["p", "It was a sunny day."],
			["p", "Tail."],
		]);
		expect(hasAIMarks(handle)).toBe(false);
		expect(handle.editor.getOption(AIChatPlugin, "open")).toBe(false);

		await handle.unmount();
	});

	test("Stop cancels the backend stream, keeps the partial answer, and Discard removes it", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor();

		await openAtEndOf(handle, 0);
		await clickMenuItem("Add a summary");
		const [request] = backend.requests;
		await stream(request, "Partial");

		const stopButton = Array.from(handle.host.querySelectorAll("button")).find(
			(button) => button.textContent?.includes("Stop"),
		);
		if (!stopButton) throw new Error("the loading bar has no Stop button");
		await run(() => (stopButton as unknown as HTMLElement).click());

		expect(request.cancelled()).toBe(true);
		expect(chatOf(handle).status).toBe("ready");
		expect(handle.editor.getOption(AIChatPlugin, "streaming")).toBe(false);
		await stream(request, " ignored");
		expect(blockTexts(handle)).toEqual([
			["p", "Hello there."],
			["p", "Partial"],
			[KEYS.aiChat, ""],
			["p", "Tail."],
		]);
		expect(menuItems()).toEqual(["Accept", "Discard", "Try again"]);

		await clickMenuItem("Discard");
		expect(blockTexts(handle)).toEqual([
			["p", "Hello there."],
			["p", "Tail."],
		]);
		expect(hasAIMarks(handle)).toBe(false);

		await handle.unmount();
	});

	test("Try again resends the same prompt and replaces the inserted answer", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor();

		await openAtEndOf(handle, 0);
		await clickMenuItem("Continue writing");
		const [first] = backend.requests;
		await stream(first, "First answer.");
		await run(() => first.finish());

		await clickMenuItem("Try again");
		const second = backend.requests[1];
		expect(second.messages).toEqual(first.messages);
		await stream(second, "Second answer.");
		await run(() => second.finish());

		expect(blockTexts(handle)).toEqual([
			["p", "Hello there."],
			["p", "Second answer."],
			[KEYS.aiChat, ""],
			["p", "Tail."],
		]);
		expect(chatOf(handle).messages.map((message) => message.role)).toEqual([
			"user",
			"assistant",
		]);

		await handle.unmount();
	});

	test("a selection streams into the preview and Replace selection swaps it in", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor();

		await selectAndOpen(handle);
		expect(menuItems()).toEqual([
			"Improve writing",
			"Emojify",
			"Make longer",
			"Make shorter",
			"Fix spelling & grammar",
			"Simplify language",
			"Summarize in bullets",
		]);

		await clickMenuItem("Improve writing");
		const [request] = backend.requests;
		expect(request.messages[0].content).toEndWith(
			"Improve the writing about <Selection>",
		);
		expect(request.messages[0].content).toContain(
			"<Selection>\nHello\n\n</Selection>",
		);
		expect(handle.editor.getOption(AIChatPlugin, "mode")).toBe("chat");
		expect(window.document.body.textContent).toContain("Thinking...");

		await stream(request, "Howdy");
		expect(window.document.body.textContent).toContain("Howdy");
		expect(blockTexts(handle)[0]).toEqual(["p", "Hello there."]);

		await run(() => request.finish());
		expect(menuItems()).toEqual([
			"Replace selection",
			"Insert below",
			"Discard",
			"Try again",
		]);

		await clickMenuItem("Replace selection");
		expect(blockTexts(handle)).toEqual([
			["p", "Howdy there."],
			["p", "Tail."],
		]);

		await handle.unmount();
	});

	test("Insert below puts the previewed answer under the selected block", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor();

		await selectAndOpen(handle);
		await clickMenuItem("Emojify");
		const [request] = backend.requests;
		await stream(request, "Hello 👋");
		await run(() => request.finish());
		expect(handle.editor.getOption(AIChatPlugin, "toolName")).toBe("generate");

		await clickMenuItem("Insert below");
		expect(blockTexts(handle)).toEqual([
			["p", "Hello there."],
			["p", "Hello 👋"],
			["p", "Tail."],
		]);

		await handle.unmount();
	});

	test("Enter sends a typed question that matches no item through the template and clears the input", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor();

		await openAtEndOf(handle, 0);
		const input = window.document.body.querySelector("[cmdk-input]");
		if (!input) throw new Error("the AI menu has no input");
		await type(input, "Why is the sky blue?");
		expect(menuItems()).toEqual([]);
		await pressEnter(input);
		const [request] = backend.requests;
		expect(request.messages[0].content).toBe(
			PROMPT_TEMPLATES.userDefault
				.replace("{block}", "Hello there.\n")
				.replace("{prompt}", "Why is the sky blue?"),
		);

		await run(() => request.finish());
		expect(
			(window.document.body.querySelector("[cmdk-input]") as { value?: string })
				?.value,
		).toBe("");

		await handle.unmount();
	});

	test("Esc stops an insert stream", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor();

		await openAtEndOf(handle, 0);
		await clickMenuItem("Continue writing");
		const [request] = backend.requests;
		await stream(request, "Going");

		await run(() =>
			window.document.dispatchEvent(
				new window.KeyboardEvent("keydown", {
					key: "Escape",
					code: "Escape",
					bubbles: true,
				}),
			),
		);

		expect(request.cancelled()).toBe(true);
		expect(chatOf(handle).status).toBe("ready");
		expect(menuItems()).toEqual(["Accept", "Discard", "Try again"]);

		await handle.unmount();
	});

	test("copilot stays quiet while the AI menu streams", async () => {
		const backend = streamingBackend();
		let completions = 0;
		backend.aiState.chatComplete = async () => {
			completions += 1;
			return { choices: [] } as unknown as IResponse;
		};
		useFakeBackend(backend);
		const handle = await mountEditor();

		await openAtEndOf(handle, 0);
		await clickMenuItem("Continue writing");
		await stream(backend.requests[0], "Some text");

		expect(await triggerCopilotSuggestion(handle.editor)).toBe(false);
		expect(completions).toBe(0);

		await handle.unmount();
	});
});

describe("useChat", () => {
	test("publishes the SDK helpers plus the copilot loading flag to the plugin", async () => {
		useFakeBackend(streamingBackend());
		const handle = await mountEditor();

		const chat = chatOf(handle);
		for (const helper of [
			"sendMessage",
			"regenerate",
			"stop",
			"setMessages",
		] as const)
			expect(typeof chat[helper]).toBe("function");
		expect(chat.status).toBe("ready");
		expect(chat.messages).toEqual([]);
		expect(chat.isLoading).toBe(false);

		await handle.unmount();
	});

	test("each editor owns its conversation", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const busy = await mountEditor();
		const idle = await mountEditor();

		await openAtEndOf(busy, 0);
		await clickMenuItem("Continue writing");
		await stream(backend.requests[0], "Only here.");

		expect(chatOf(busy).status).toBe("streaming");
		expect(chatOf(idle).status).toBe("ready");
		expect(chatOf(idle).messages).toEqual([]);
		expect(blockTexts(idle)).toEqual([
			["p", "Hello there."],
			["p", "Tail."],
		]);

		await busy.unmount();
		await idle.unmount();
	});

	test("requests use the backend and app scope current at send time", async () => {
		const before = streamingBackend();
		useFakeBackend(before);
		const handle = await mountEditor({ appId: "app-old" });

		const after = streamingBackend();
		useFakeBackend(after);
		await handle.render("app-new");
		await openAtEndOf(handle, 0);
		await clickMenuItem("Continue writing");

		expect(before.requests).toHaveLength(0);
		expect(after.requests.map((request) => request.appId)).toEqual(["app-new"]);

		await handle.unmount();
	});

	test("publishes once per state change and settles after the stream ends", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor();
		const setOption = handle.editor.setOption.bind(handle.editor);
		let publishes = 0;
		handle.editor.setOption = ((plugin, key, ...rest) => {
			if (key === "chat") publishes += 1;
			return (setOption as (...args: unknown[]) => unknown)(
				plugin,
				key,
				...rest,
			);
		}) as typeof handle.editor.setOption;

		await openAtEndOf(handle, 0);
		await clickMenuItem("Continue writing");
		const [request] = backend.requests;
		for (const delta of ["a", "b", "c", "d", "e"]) await stream(request, delta);
		await run(() => request.finish());
		const afterStream = publishes;
		await settle(6);

		expect(afterStream).toBeGreaterThanOrEqual(5);
		expect(afterStream).toBeLessThanOrEqual(20);
		expect(publishes).toBe(afterStream);

		await handle.unmount();
	});

	test("unmounting mid-stream cancels the backend stream", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor();

		await openAtEndOf(handle, 0);
		await clickMenuItem("Continue writing");
		const [request] = backend.requests;
		await stream(request, "Half");

		await handle.unmount();
		await settle(2);

		expect(request.cancelled()).toBe(true);
	});
});

describe("markdown code stays verbatim on every editable path", () => {
	type TextNode = { text?: string; children?: TextNode[] };

	const texts = (nodes: readonly unknown[]): string[] =>
		(nodes as readonly TextNode[]).flatMap((node) =>
			typeof node.text === "string" ? [node.text] : texts(node.children ?? []),
		);

	const codeLines = ({ editor }: EditorHandle) =>
		Array.from(
			editor.api.nodes({ at: [], match: { type: KEYS.codeLine } }),
			([node]) => NodeApi.string(node),
		);

	const inlineCode = ({ editor }: EditorHandle) =>
		Array.from(
			editor.api.nodes({ at: [], match: (node) => !!node[KEYS.code] }),
			([node]) => NodeApi.string(node),
		);

	const freshEditor = () => {
		const editor = createPlateEditor({
			plugins: createEditorKit(),
			value: [{ type: "p", children: [{ text: "" }] }],
		});
		editor.tf.select(editor.api.start([0]));
		return editor;
	};

	for (const { title, markdown, verbatim } of CODE_MARKUP_FIXTURES) {
		test(`Import from Markdown: ${title}`, () => {
			const editor = freshEditor();
			editor.tf.insertNodes(deserializeMarkdownFile(editor, markdown));
			for (const text of verbatim)
				expect(texts(editor.children)).toContain(text);
		});

		test(`pasted markdown text: ${title}`, () => {
			const editor = freshEditor();
			const data: Record<string, string> = { "text/plain": markdown };
			editor.tf.insertData({
				getData: (format: string) => data[format] ?? "",
				setData: () => {},
				types: Object.keys(data),
				files: [],
				items: [],
			} as unknown as DataTransfer);
			for (const text of verbatim)
				expect(texts(editor.children)).toContain(text);
		});
	}

	test("an inserted answer streamed in chunks that split the fence and its tags", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor();

		await openAtEndOf(handle, 0);
		await clickMenuItem("Continue writing");
		const [request] = backend.requests;

		await stream(request, "Markup:\n\n```html\n<div cla");
		expect(codeLines(handle)).toEqual(["<div cla"]);
		await stream(request, 'ss="card" hid');
		expect(codeLines(handle)).toEqual(['<div class="card" hid']);
		await stream(request, "den>\n  <input disabled><br>\n</d");
		expect(codeLines(handle)).toEqual([
			'<div class="card" hidden>',
			"  <input disabled><br>",
			"</d",
		]);
		await stream(request, "iv>\n```\n\n- item with `<label f");
		await stream(request, 'or="a">`\n\n> quote with `<img src=x>`');
		await run(() => request.finish());
		await clickMenuItem("Accept");

		expect(codeLines(handle)).toEqual([
			'<div class="card" hidden>',
			"  <input disabled><br>",
			"</div>",
		]);
		expect(inlineCode(handle)).toEqual(['<label for="a">', "<img src=x>"]);
		expect(blockTexts(handle)).toEqual([
			["p", "Hello there."],
			["p", "Markup:"],
			["code_block", '<div class="card" hidden>  <input disabled><br></div>'],
			["p", 'item with <label for="a">'],
			["blockquote", "quote with <img src=x>"],
			["p", "Tail."],
		]);

		await handle.unmount();
	});

	test("the chat preview and Replace selection", async () => {
		const backend = streamingBackend();
		useFakeBackend(backend);
		const handle = await mountEditor();

		await selectAndOpen(handle);
		await clickMenuItem("Improve writing");
		const [request] = backend.requests;
		await stream(request, '```html\n<p class="a">');
		await stream(request, "Hi<br></p>\n```\n\nSay `<b for=x>`");
		const preview = handle.editor.getOption(AIChatPlugin, "aiEditor");
		expect(texts(preview?.children ?? [])).toEqual([
			'<p class="a">Hi<br></p>',
			"Say ",
			"<b for=x>",
		]);
		expect(window.document.body.textContent).toContain(
			'<p class="a">Hi<br></p>',
		);

		await run(() => request.finish());
		await clickMenuItem("Replace selection");
		expect(codeLines(handle)).toEqual(['<p class="a">Hi<br></p>']);
		expect(inlineCode(handle)).toEqual(["<b for=x>"]);
		expect(blockTexts(handle)).toEqual([
			["code_block", '<p class="a">Hi<br></p>'],
			["p", "Say <b for=x> there."],
			["p", "Tail."],
		]);

		await handle.unmount();
	});
});
