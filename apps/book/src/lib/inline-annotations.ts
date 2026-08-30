import type { ReadingBookmark, ReadingComment } from "./reading-progress";

export type InlineAnnotationKind = "bookmark" | "comment";

export interface InlineAnnotationOpenRequest {
	kind: InlineAnnotationKind;
	id: string;
}

interface InlineAnnotation {
	kind: InlineAnnotationKind;
	record: ReadingBookmark | ReadingComment;
}

interface TextPoint {
	node: Text;
	offset: number;
}

interface IndexedSectionText {
	text: string;
	points: TextPoint[];
}

interface LocatedAnnotation extends InlineAnnotation {
	anchor: HTMLElement;
	range?: Range;
}

interface AnnotationGroup {
	bookmark: LocatedAnnotation[];
	comment: LocatedAnnotation[];
}

interface WritableHighlightRegistry {
	delete(name: string): boolean;
	set(name: string, highlight: Highlight): void;
}

const BOOKMARK_HIGHLIGHT = "flowbook-bookmarks";
const COMMENT_HIGHLIGHT = "flowbook-comments";
const MARKER_SELECTOR = "[data-flowbook-annotation-marker]";
const ANNOTATED_SELECTOR = "[data-flowbook-annotated-block]";
const TEXT_BLOCK_SELECTOR = "p, li, blockquote, td, th, figcaption, h2, h3, h4";
const MARKER_ANCHOR_SELECTOR =
	"p, li, blockquote, td, th, figcaption, .sl-heading-wrapper";
const SKIPPED_TEXT_SELECTOR =
	"astro-island, script, style, textarea, input, select, button, pre, code, [data-flowbook-annotation-marker]";

function normalizeQuote(value: string): string {
	return value.replace(/\s+/g, " ").trim();
}

function writableHighlightRegistry(): WritableHighlightRegistry | undefined {
	if (typeof CSS === "undefined" || !("highlights" in CSS)) return undefined;
	return CSS.highlights as HighlightRegistry & WritableHighlightRegistry;
}

function removeOwnedPresentation(): void {
	const registry = writableHighlightRegistry();
	registry?.delete(BOOKMARK_HIGHLIGHT);
	registry?.delete(COMMENT_HIGHLIGHT);

	for (const marker of document.querySelectorAll(MARKER_SELECTOR)) {
		marker.remove();
	}
	for (const block of document.querySelectorAll<HTMLElement>(
		ANNOTATED_SELECTOR,
	)) {
		block.classList.remove("flowbook-annotated-block");
		block.removeAttribute("data-flowbook-annotated-block");
		block.removeAttribute("data-flowbook-has-bookmark");
		block.removeAttribute("data-flowbook-has-comment");
	}
}

function sectionRange(
	content: HTMLElement,
	headingId: string,
): { range: Range; heading?: HTMLElement } {
	const headings = Array.from(
		content.querySelectorAll<HTMLElement>("h2[id], h3[id]"),
	);
	const headingIndex = headings.findIndex(
		(heading) => heading.id === headingId,
	);
	const heading = headingIndex >= 0 ? headings[headingIndex] : undefined;
	const boundary = headingIndex >= 0 ? headings[headingIndex + 1] : headings[0];
	const range = document.createRange();

	if (heading) range.setStartBefore(heading);
	else range.setStart(content, 0);

	if (boundary && boundary !== heading) range.setEndBefore(boundary);
	else range.setEnd(content, content.childNodes.length);

	return { range, heading };
}

function appendMappedCharacter(
	indexed: IndexedSectionText,
	character: string,
	node: Text,
	offset: number,
): void {
	indexed.text += character;
	indexed.points.push({ node, offset });
}

function indexSectionText(
	content: HTMLElement,
	scope: Range,
): IndexedSectionText {
	const indexed: IndexedSectionText = { text: "", points: [] };
	const walker = document.createTreeWalker(content, 4);
	let previousBlock: Element | null = null;
	let forceSeparator = false;
	let current = walker.nextNode();

	while (current) {
		const node = current as Text;
		const parent = node.parentElement;
		let intersects = false;
		try {
			intersects = scope.intersectsNode(node);
		} catch {
			intersects = false;
		}

		if (!intersects || !parent || parent.closest(SKIPPED_TEXT_SELECTOR)) {
			if (intersects && node.data.trim()) forceSeparator = true;
			current = walker.nextNode();
			continue;
		}

		const block = parent.closest(TEXT_BLOCK_SELECTOR);
		if (
			indexed.text &&
			!indexed.text.endsWith(" ") &&
			(forceSeparator || (previousBlock && block !== previousBlock))
		) {
			appendMappedCharacter(indexed, " ", node, 0);
		}
		forceSeparator = false;

		for (let offset = 0; offset < node.data.length; offset += 1) {
			const character = node.data[offset];
			if (/\s/u.test(character)) {
				if (indexed.text && !indexed.text.endsWith(" ")) {
					appendMappedCharacter(indexed, " ", node, offset);
				}
			} else {
				appendMappedCharacter(indexed, character, node, offset);
			}
		}
		previousBlock = block;
		current = walker.nextNode();
	}

	return indexed;
}

function matchingRange(
	content: HTMLElement,
	annotation: ReadingBookmark | ReadingComment,
): Range | undefined {
	const quote = normalizeQuote(annotation.quote ?? "");
	if (!quote) return undefined;
	const { range: scope } = sectionRange(content, annotation.headingId);
	const indexed = indexSectionText(content, scope);
	const matches: number[] = [];
	let match = indexed.text.indexOf(quote);
	while (match >= 0) {
		matches.push(match);
		match = indexed.text.indexOf(quote, match + 1);
	}
	if (matches.length === 0) return undefined;
	if (
		matches.length > 1 &&
		(typeof annotation.sectionProgress !== "number" ||
			!Number.isFinite(annotation.sectionProgress))
	) {
		return undefined;
	}

	const target =
		Math.min(1, Math.max(0, annotation.sectionProgress ?? 0)) *
		indexed.text.length;
	const startIndex = matches.reduce((nearest, candidate) =>
		Math.abs(candidate + quote.length / 2 - target) <
		Math.abs(nearest + quote.length / 2 - target)
			? candidate
			: nearest,
	);
	const start = indexed.points[startIndex];
	const end = indexed.points[startIndex + quote.length - 1];
	if (!start || !end) return undefined;

	const range = document.createRange();
	range.setStart(start.node, start.offset);
	range.setEnd(end.node, Math.min(end.node.length, end.offset + 1));
	return range;
}

function markerAnchorForRange(range: Range): HTMLElement | undefined {
	const parent =
		range.startContainer.nodeType === Node.TEXT_NODE
			? range.startContainer.parentElement
			: (range.startContainer as Element);
	if (!parent || parent.closest("astro-island, pre, code")) return undefined;
	const heading = parent.closest<HTMLElement>("h2, h3, h4");
	if (heading) {
		return heading.closest<HTMLElement>(".sl-heading-wrapper") ?? heading;
	}
	return parent.closest<HTMLElement>(MARKER_ANCHOR_SELECTOR) ?? undefined;
}

function fallbackAnchor(
	content: HTMLElement,
	annotation: ReadingBookmark | ReadingComment,
): HTMLElement {
	const heading = document.getElementById(annotation.headingId);
	if (heading instanceof HTMLElement) {
		return heading.closest<HTMLElement>(".sl-heading-wrapper") ?? heading;
	}
	return content.querySelector<HTMLElement>(MARKER_ANCHOR_SELECTOR) ?? content;
}

function locateAnnotation(
	content: HTMLElement,
	annotation: InlineAnnotation,
): LocatedAnnotation {
	const range = matchingRange(content, annotation.record);
	return {
		...annotation,
		range,
		anchor:
			(range ? markerAnchorForRange(range) : undefined) ??
			fallbackAnchor(content, annotation.record),
	};
}

function markerLabel(
	kind: InlineAnnotationKind,
	annotations: LocatedAnnotation[],
): string {
	const count = annotations.length;
	const quote = annotations[0]?.record.quote;
	const location = quote
		? `“${quote.slice(0, 72)}${quote.length > 72 ? "…" : ""}”`
		: annotations[0]?.record.headingText || "this section";
	return `Open ${count > 1 ? `${count} ` : ""}${kind}${count > 1 ? "s" : ""} on ${location}`;
}

function createMarkerButton(
	kind: InlineAnnotationKind,
	annotations: LocatedAnnotation[],
	onOpen: (request: InlineAnnotationOpenRequest) => void,
): HTMLButtonElement {
	const button = document.createElement("button");
	button.type = "button";
	button.className = `flowbook-inline-annotation flowbook-inline-annotation--${kind}`;
	button.setAttribute("aria-label", markerLabel(kind, annotations));
	button.title = markerLabel(kind, annotations);
	if (annotations.length > 1) button.dataset.count = String(annotations.length);

	const glyph = document.createElement("i");
	glyph.setAttribute("aria-hidden", "true");
	glyph.className =
		kind === "bookmark" ? "flowbook-bookmark-glyph" : "flowbook-comment-glyph";
	button.append(glyph);
	button.addEventListener("click", () =>
		onOpen({ kind, id: annotations[0].record.id }),
	);
	return button;
}

function registerHighlights(
	bookmarkRanges: Range[],
	commentRanges: Range[],
): void {
	const registry = writableHighlightRegistry();
	if (!registry || typeof Highlight === "undefined") return;
	if (bookmarkRanges.length > 0) {
		registry.set(BOOKMARK_HIGHLIGHT, new Highlight(...bookmarkRanges));
	}
	if (commentRanges.length > 0) {
		const comments = new Highlight(...commentRanges);
		comments.priority = 1;
		registry.set(COMMENT_HIGHLIGHT, comments);
	}
}

export function installInlineAnnotations(
	content: HTMLElement,
	bookmarks: readonly ReadingBookmark[],
	comments: readonly ReadingComment[],
	onOpen: (request: InlineAnnotationOpenRequest) => void,
): () => void {
	removeOwnedPresentation();
	const located = [
		...bookmarks.map((record) =>
			locateAnnotation(content, { kind: "bookmark", record }),
		),
		...comments.map((record) =>
			locateAnnotation(content, { kind: "comment", record }),
		),
	];
	const bookmarkRanges = located
		.filter((annotation) => annotation.kind === "bookmark" && annotation.range)
		.map((annotation) => annotation.range as Range);
	const commentRanges = located
		.filter((annotation) => annotation.kind === "comment" && annotation.range)
		.map((annotation) => annotation.range as Range);
	registerHighlights(bookmarkRanges, commentRanges);

	const groups = new Map<HTMLElement, AnnotationGroup>();
	for (const annotation of located) {
		const group = groups.get(annotation.anchor) ?? {
			bookmark: [],
			comment: [],
		};
		group[annotation.kind].push(annotation);
		groups.set(annotation.anchor, group);
	}

	for (const [anchor, annotations] of groups) {
		const marker = document.createElement("span");
		marker.className = "flowbook-inline-annotations";
		marker.setAttribute("data-flowbook-annotation-marker", "");
		marker.setAttribute("contenteditable", "false");
		anchor.classList.add("flowbook-annotated-block");
		anchor.setAttribute("data-flowbook-annotated-block", "");
		if (annotations.bookmark.length > 0) {
			anchor.setAttribute("data-flowbook-has-bookmark", "");
			marker.append(
				createMarkerButton("bookmark", annotations.bookmark, onOpen),
			);
		}
		if (annotations.comment.length > 0) {
			anchor.setAttribute("data-flowbook-has-comment", "");
			marker.append(createMarkerButton("comment", annotations.comment, onOpen));
		}
		anchor.append(marker);
	}

	return removeOwnedPresentation;
}
