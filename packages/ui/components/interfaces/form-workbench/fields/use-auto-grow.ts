import { type RefObject, useEffect, useLayoutEffect } from "react";

function fit(node: HTMLTextAreaElement) {
	node.style.height = "auto";
	node.style.height = `${node.scrollHeight + 2}px`;
}

/**
 * Calls `refit` once the document's web fonts have loaded, and after every later load: the first fit measured
 * the fallback face, and a value that only wraps in the web font would keep a line cut off. Returns the stop.
 */
function onFontsLoaded(doc: Document, refit: () => void) {
	const fonts: FontFaceSet | undefined = doc.fonts;
	if (!fonts) return () => {};
	let live = true;
	const loaded = () => {
		if (live) refit();
	};
	fonts.ready.then(loaded, () => {});
	fonts.addEventListener("loadingdone", loaded);
	return () => {
		live = false;
		fonts.removeEventListener("loadingdone", loaded);
	};
}

/** Refits when the box's width changes (the rail narrows, a wrap moves), not on its own height changes. */
function onWidthChange(node: HTMLTextAreaElement, refit: () => void) {
	if (typeof ResizeObserver === "undefined") return () => {};
	let width = node.clientWidth;
	const observer = new ResizeObserver(() => {
		if (node.clientWidth === width) return;
		width = node.clientWidth;
		refit();
	});
	observer.observe(node);
	return () => observer.disconnect();
}

/**
 * A text box that is one line until its text needs more (SURFACE §4): the height follows the content, plus the
 * 2 px of border, after every change, when the box itself is resized (the rail narrows, a wrap moves), when
 * the pointer flips between fine and coarse (`touch`: padding and type size change, the width does not) and
 * when the web font arrives (its glyphs are wider or narrower than the fallback's).
 */
export function useAutoGrow(
	element: RefObject<HTMLTextAreaElement | null>,
	text: string,
	touch: boolean,
) {
	// biome-ignore lint/correctness/useExhaustiveDependencies: refit whenever the text or the control metrics change
	useLayoutEffect(() => {
		if (element.current) fit(element.current);
	}, [element, text, touch]);
	useEffect(() => {
		const node = element.current;
		if (!node) return;
		const refit = () => fit(node);
		const stopWidth = onWidthChange(node, refit);
		const stopFonts = onFontsLoaded(node.ownerDocument, refit);
		return () => {
			stopWidth();
			stopFonts();
		};
	}, [element]);
}
