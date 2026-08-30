import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";

const chapter = {
	entryId: "introduction",
	path: "/introduction/",
	title: "Introduction",
	label: "Introduction",
};

let browserWindow: Window;
let root: Root | undefined;
let act: typeof import("react")["act"];
let createRoot: typeof import("react-dom/client")["createRoot"];
let ReadingExperience: typeof import("./ReadingExperience")["default"];

function expose(name: string, value: unknown) {
	Object.defineProperty(globalThis, name, {
		configurable: true,
		writable: true,
		value,
	});
}

async function settle() {
	await act(async () => {
		await new Promise<void>((resolve) => setTimeout(resolve, 20));
	});
}

function selectPassage(text = "Selected passage", passageId = "passage") {
	const passage = document.querySelector(`#${passageId}`)?.firstChild;
	if (!passage) throw new Error("Passage fixture is missing");
	const range = document.createRange();
	range.setStart(passage, 0);
	range.setEnd(passage, text.length);
	const selection = window.getSelection();
	selection?.removeAllRanges();
	selection?.addRange(range);
	document.dispatchEvent(new Event("selectionchange"));
}

beforeEach(async () => {
	browserWindow = new Window({
		url: "https://book.flow-like.test/introduction/",
	});
	for (const [name, value] of Object.entries({
		Error,
		TypeError,
		SyntaxError,
		RangeError,
		ReferenceError,
	})) {
		Object.defineProperty(browserWindow, name, {
			configurable: true,
			value,
		});
	}
	const bindings: Record<string, unknown> = {
		window: browserWindow,
		document: browserWindow.document,
		navigator: browserWindow.navigator,
		Node: browserWindow.Node,
		Element: browserWindow.Element,
		HTMLElement: browserWindow.HTMLElement,
		HTMLDialogElement: browserWindow.HTMLDialogElement,
		Event: browserWindow.Event,
		InputEvent: browserWindow.InputEvent,
		PointerEvent: browserWindow.PointerEvent,
		MouseEvent: browserWindow.MouseEvent,
		KeyboardEvent: browserWindow.KeyboardEvent,
		CustomEvent: browserWindow.CustomEvent,
		MutationObserver: browserWindow.MutationObserver,
		performance: browserWindow.performance,
		history: browserWindow.history,
		location: browserWindow.location,
		crypto: browserWindow.crypto,
		getComputedStyle: browserWindow.getComputedStyle.bind(browserWindow),
		requestAnimationFrame:
			browserWindow.requestAnimationFrame.bind(browserWindow),
		cancelAnimationFrame:
			browserWindow.cancelAnimationFrame.bind(browserWindow),
		indexedDB: undefined,
	};
	for (const [name, value] of Object.entries(bindings)) expose(name, value);
	({ act } = await import("react"));
	({ createRoot } = await import("react-dom/client"));
	({ default: ReadingExperience } = await import("./ReadingExperience"));
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;

	document.body.innerHTML = `
		<div class="main-pane">
			<main data-pagefind-body>
				<h1 id="_top">Introduction</h1>
				<div class="sl-markdown-content">
					<h2 id="first-section">First section</h2>
					<p id="passage">Selected passage for a private comment.</p>
					<p id="passage-two">Another selected passage in this section.</p>
				</div>
				<div id="reader-root"></div>
			</main>
		</div>`;
	const container = document.querySelector("#reader-root");
	if (!container) throw new Error("Reader root fixture is missing");
	const mountedRoot = createRoot(container);
	root = mountedRoot;
	await act(async () => {
		mountedRoot.render(
			<ReadingExperience
				editionId="test-edition"
				chapters={[chapter]}
				currentEntryId={chapter.entryId}
				currentPath={chapter.path}
				currentTitle={chapter.title}
				isReadingPage
				isLandingPage={false}
			/>,
		);
	});
	await settle();
});

afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	root = undefined;
	await browserWindow.happyDOM.close();
});

describe("ReadingExperience interactions", () => {
	test("portals controls above the Starlight pane and opens the progress panel", async () => {
		const dock = document.querySelector<HTMLElement>(".flowbook-reader-dock");
		expect(dock).not.toBeNull();
		expect(dock?.closest(".main-pane")).toBeNull();

		const progress = dock?.querySelector<HTMLButtonElement>(
			".flowbook-reader-dock__progress",
		);
		await act(async () => progress?.click());
		expect(document.querySelector("#flowbook-reader-panel")).not.toBeNull();
	});

	test("shows selected-text actions while Comments is open", async () => {
		const commentDock = Array.from(
			document.querySelectorAll<HTMLButtonElement>(
				".flowbook-reader-dock button",
			),
		).find((button) => button.textContent?.includes("Comment"));
		await act(async () => commentDock?.click());
		expect(document.querySelector("#flowbook-reader-panel")).not.toBeNull();

		selectPassage();
		await settle();
		const toolbar = document.querySelector(".flowbook-selection-tools");
		expect(toolbar).not.toBeNull();
		const addNote = Array.from(
			toolbar?.querySelectorAll<HTMLButtonElement>("button") ?? [],
		).find((button) => button.textContent?.includes("Add note"));
		await act(async () => addNote?.click());
		await settle();

		expect(document.querySelector("#flowbook-reader-panel")).not.toBeNull();
		expect(
			document.querySelector(".flowbook-comments blockquote")?.textContent,
		).toContain("Selected passage");
		expect(document.activeElement?.id).toBe("flowbook-comment-draft");
		expect(document.querySelector(".flowbook-selection-tools")).toBeNull();
	});

	test("keeps selected text through composing and saves the comment", async () => {
		selectPassage();
		await settle();
		const toolbar = document.querySelector(".flowbook-selection-tools");
		expect(toolbar).not.toBeNull();
		const comment = Array.from(toolbar?.querySelectorAll("button") ?? []).find(
			(button) => button.textContent?.includes("Add note"),
		);
		await act(async () => comment?.click());

		const quote = document.querySelector(".flowbook-comments blockquote");
		expect(quote?.textContent).toContain("Selected passage");

		const overview = Array.from(
			document.querySelectorAll<HTMLButtonElement>(
				".flowbook-reader-panel > nav button",
			),
		).find((button) => button.textContent?.includes("Overview"));
		await act(async () => overview?.click());
		window.getSelection()?.removeAllRanges();
		const comments = Array.from(
			document.querySelectorAll<HTMLButtonElement>(
				".flowbook-reader-panel > nav button",
			),
		).find((button) => button.textContent?.includes("Comments"));
		await act(async () => comments?.click());
		expect(
			document.querySelector(".flowbook-comments blockquote")?.textContent,
		).toContain("Selected passage");

		const textarea = document.querySelector<HTMLTextAreaElement>(
			"#flowbook-comment-draft",
		);
		if (!textarea) throw new Error("Comment textarea is missing");
		const valueSetter = Object.getOwnPropertyDescriptor(
			browserWindow.HTMLTextAreaElement.prototype,
			"value",
		)?.set;
		await act(async () => {
			valueSetter?.call(textarea, "Remember this design decision");
			textarea.dispatchEvent(
				new InputEvent("input", {
					bubbles: true,
					data: "Remember this design decision",
					inputType: "insertText",
				}),
			);
		});
		await settle();
		const submit = document.querySelector<HTMLButtonElement>(
			'.flowbook-comments button[type="submit"]',
		);
		expect(submit?.disabled).toBe(false);
		await act(async () => submit?.click());
		await settle();

		expect(
			document.querySelector(".flowbook-comment-list article > p")?.textContent,
		).toBe("Remember this design decision");
		expect(
			document.querySelector(".flowbook-inline-annotation--comment"),
		).not.toBeNull();
	});

	test("saves a selected-text bookmark even when IndexedDB is unavailable", async () => {
		selectPassage();
		await settle();
		const save = Array.from(
			document.querySelectorAll<HTMLButtonElement>(
				".flowbook-selection-tools button",
			),
		).find((button) => button.textContent?.includes("Bookmark"));
		await act(async () => save?.click());
		await settle();

		const inlineBookmark = document.querySelector<HTMLButtonElement>(
			".flowbook-inline-annotation--bookmark",
		);
		expect(inlineBookmark).not.toBeNull();
		await act(async () => inlineBookmark?.click());
		await settle();
		expect(document.querySelector(".flowbook-saved-quote")).not.toBeNull();
		expect(
			document.querySelector(
				".flowbook-saved-list article[data-flowbook-annotation-id].is-targeted",
			),
		).not.toBeNull();
		expect(
			document.querySelector(".flowbook-reader-announcement")?.textContent,
		).toContain("session");
	});

	test("keeps two selected-text bookmarks in the same section", async () => {
		selectPassage();
		await settle();
		let bookmark = Array.from(
			document.querySelectorAll<HTMLButtonElement>(
				".flowbook-selection-tools button",
			),
		).find((button) => button.textContent?.includes("Bookmark"));
		await act(async () => bookmark?.click());

		selectPassage("Another selected passage", "passage-two");
		await settle();
		bookmark = Array.from(
			document.querySelectorAll<HTMLButtonElement>(
				".flowbook-selection-tools button",
			),
		).find((button) => button.textContent?.includes("Bookmark"));
		await act(async () => bookmark?.click());

		const progress = document.querySelector<HTMLButtonElement>(
			".flowbook-reader-dock__progress",
		);
		await act(async () => progress?.click());
		const bookmarksTab = Array.from(
			document.querySelectorAll<HTMLButtonElement>(
				".flowbook-reader-panel > nav button",
			),
		).find((button) => button.textContent?.includes("Bookmarks 2"));
		await act(async () => bookmarksTab?.click());

		expect(document.querySelectorAll(".flowbook-saved-quote").length).toBe(2);
	});
});
