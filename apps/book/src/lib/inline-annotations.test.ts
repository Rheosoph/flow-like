import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import {
	type InlineAnnotationOpenRequest,
	installInlineAnnotations,
} from "./inline-annotations";
import type { ReadingBookmark, ReadingComment } from "./reading-progress";

class FakeHighlight {
	priority = 0;
	type = "highlight";
	readonly ranges: AbstractRange[];

	constructor(...ranges: AbstractRange[]) {
		this.ranges = ranges;
	}
}

let browserWindow: Window;
let highlightRegistry: Map<string, FakeHighlight>;
let cleanup: (() => void) | undefined;
const originalCSS = globalThis.CSS;
const originalHighlight = globalThis.Highlight;

function expose(name: string, value: unknown) {
	Object.defineProperty(globalThis, name, {
		configurable: true,
		writable: true,
		value,
	});
}

function bookmark(id: string, quote = "Selected passage"): ReadingBookmark {
	return {
		id,
		editionId: "edition",
		entryId: "introduction",
		path: "/introduction/",
		title: "Introduction",
		headingId: "first-section",
		headingText: "First section",
		scrollY: 100,
		headingOffset: 10,
		sectionProgress: 0.4,
		percent: 0.2,
		quote,
		createdAt: "2026-01-01T00:00:00.000Z",
	};
}

function comment(id: string): ReadingComment {
	return {
		...bookmark(id),
		body: "Remember why this matters",
		updatedAt: "2026-01-01T00:00:00.000Z",
	};
}

beforeEach(() => {
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
	highlightRegistry = new Map();
	for (const [name, value] of Object.entries({
		window: browserWindow,
		document: browserWindow.document,
		Node: browserWindow.Node,
		Element: browserWindow.Element,
		HTMLElement: browserWindow.HTMLElement,
		MouseEvent: browserWindow.MouseEvent,
		CSS: { highlights: highlightRegistry },
		Highlight: FakeHighlight,
	})) {
		expose(name, value);
	}
	document.body.innerHTML = `
		<main data-pagefind-body>
			<h1 id="_top">Introduction</h1>
			<div class="sl-markdown-content">
				<div class="sl-heading-wrapper"><h2 id="first-section">First section</h2></div>
				<p id="passage">Selected <em>passage</em> for a private comment.</p>
				<div class="sl-heading-wrapper"><h2 id="second-section">Second section</h2></div>
				<p>Selected passage somewhere else.</p>
			</div>
		</main>`;
});

afterEach(async () => {
	cleanup?.();
	cleanup = undefined;
	await browserWindow.happyDOM.close();
	expose("CSS", originalCSS);
	expose("Highlight", originalHighlight);
});

describe("inline annotations", () => {
	test("highlights a quote across inline markup and opens its marker", () => {
		const opened: InlineAnnotationOpenRequest[] = [];
		const content = document.querySelector<HTMLElement>(".sl-markdown-content");
		if (!content) throw new Error("Content fixture is missing");

		cleanup = installInlineAnnotations(
			content,
			[bookmark("bookmark-1")],
			[],
			(request) => opened.push(request),
		);

		const highlight = highlightRegistry.get("flowbook-bookmarks");
		expect(highlight?.ranges).toHaveLength(1);
		expect(highlight?.ranges[0]?.toString()).toBe("Selected passage");
		const marker = document.querySelector<HTMLButtonElement>(
			".flowbook-inline-annotation--bookmark",
		);
		expect(marker?.closest("#passage")).not.toBeNull();
		expect(marker?.parentElement?.textContent).toBe("");
		marker?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
		expect(opened).toEqual([{ kind: "bookmark", id: "bookmark-1" }]);
	});

	test("aggregates comments and cleans up owned presentation", () => {
		const content = document.querySelector<HTMLElement>(".sl-markdown-content");
		if (!content) throw new Error("Content fixture is missing");
		cleanup = installInlineAnnotations(
			content,
			[],
			[comment("comment-1"), comment("comment-2")],
			() => undefined,
		);

		expect(
			document.querySelector<HTMLButtonElement>(
				".flowbook-inline-annotation--comment",
			)?.dataset.count,
		).toBe("2");
		expect(highlightRegistry.get("flowbook-comments")?.ranges).toHaveLength(2);

		cleanup();
		cleanup = undefined;
		expect(
			document.querySelector("[data-flowbook-annotation-marker]"),
		).toBeNull();
		expect(
			document.querySelector("[data-flowbook-annotated-block]"),
		).toBeNull();
		expect(highlightRegistry.size).toBe(0);
	});

	test("falls back to the saved heading when prose changed", () => {
		const content = document.querySelector<HTMLElement>(".sl-markdown-content");
		if (!content) throw new Error("Content fixture is missing");
		cleanup = installInlineAnnotations(
			content,
			[bookmark("bookmark-legacy", "Passage removed from this edition")],
			[],
			() => undefined,
		);

		const marker = document.querySelector(
			".flowbook-inline-annotation--bookmark",
		);
		expect(marker?.closest(".sl-heading-wrapper")).not.toBeNull();
		expect(highlightRegistry.get("flowbook-bookmarks")).toBeUndefined();
	});

	test("resolves a quote in the final section", () => {
		const content = document.querySelector<HTMLElement>(".sl-markdown-content");
		if (!content) throw new Error("Content fixture is missing");
		cleanup = installInlineAnnotations(
			content,
			[
				{
					...bookmark("bookmark-last", "Selected passage somewhere else"),
					headingId: "second-section",
					headingText: "Second section",
				},
			],
			[],
			() => undefined,
		);

		expect(
			highlightRegistry.get("flowbook-bookmarks")?.ranges[0]?.toString(),
		).toBe("Selected passage somewhere else");
	});
});
