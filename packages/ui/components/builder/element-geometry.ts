export interface ElementRectangle {
	left: number;
	top: number;
	width: number;
	height: number;
}

/** Elements using display:contents have no box; measure their visible content. */
export function getElementRectangle(element: Element): ElementRectangle {
	const rect = element.getBoundingClientRect();
	const box = {
		left: rect.left,
		top: rect.top,
		width: rect.width,
		height: rect.height,
	};
	if (rect.width || rect.height) return box;
	const boxes = Array.from(element.children)
		.map(getElementRectangle)
		.filter((box) => box.width || box.height);
	if (!boxes.length) return box;
	const left = Math.min(...boxes.map((box) => box.left));
	const top = Math.min(...boxes.map((box) => box.top));
	return {
		left,
		top,
		width: Math.max(...boxes.map((box) => box.left + box.width)) - left,
		height: Math.max(...boxes.map((box) => box.top + box.height)) - top,
	};
}

/** Give an empty container a drop target without changing its page layout. */
export function getBuilderElementRectangle(element: Element): ElementRectangle {
	const box = getElementRectangle(element);
	if (
		!element.hasAttribute("data-builder-empty") ||
		element.ownerDocument.defaultView?.getComputedStyle(element).display ===
			"none"
	)
		return box;
	return { ...box, width: box.width || 48, height: box.height || 32 };
}

/** Find the visible canvas viewport, including its scroll ancestors. */
export function getCanvasViewport(element: HTMLElement): ElementRectangle {
	const view = element.ownerDocument.defaultView;
	if (!view) return { left: 0, top: 0, width: 0, height: 0 };
	let left = 0;
	let top = 0;
	let right = view.innerWidth;
	let bottom = view.innerHeight;
	for (
		let parent = element.parentElement;
		parent;
		parent = parent.parentElement
	) {
		const style = view.getComputedStyle(parent);
		const rect = parent.getBoundingClientRect();
		if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) {
			left = Math.max(left, rect.left);
			right = Math.min(right, rect.right);
		}
		if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) {
			top = Math.max(top, rect.top);
			bottom = Math.min(bottom, rect.bottom);
		}
	}
	return {
		left,
		top,
		width: Math.max(0, right - left),
		height: Math.max(0, bottom - top),
	};
}

/** Clip outlines and insertion markers while keeping actions in their own layer. */
export function getCanvasClip(element: HTMLElement): string {
	const view = element.ownerDocument.defaultView;
	if (!view) return "inset(0)";
	const rect = getCanvasViewport(element);
	return `inset(${rect.top}px ${Math.max(0, view.innerWidth - rect.left - rect.width)}px ${Math.max(0, view.innerHeight - rect.top - rect.height)}px ${rect.left}px)`;
}

export function placeElementToolbar(
	anchor: ElementRectangle,
	toolbar: { width: number; height: number },
	viewport: ElementRectangle,
): ElementRectangle & {
	maxWidth: number;
	maxHeight: number;
	visible: boolean;
} {
	const margin = 6;
	const maxWidth = Math.max(0, viewport.width - margin * 2);
	const maxHeight = Math.max(0, viewport.height - margin * 2);
	const width = Math.min(toolbar.width, maxWidth);
	const height = Math.min(toolbar.height, maxHeight);
	const topLimit = viewport.top + margin;
	const bottomLimit = viewport.top + viewport.height - margin;
	const above = anchor.top - height - margin;
	const below = anchor.top + anchor.height + margin;
	const desiredTop =
		above >= topLimit
			? above
			: below + height <= bottomLimit
				? below
				: anchor.top + margin;
	return {
		left: Math.max(
			viewport.left + margin,
			Math.min(anchor.left, viewport.left + viewport.width - margin - width),
		),
		top: Math.max(topLimit, Math.min(desiredTop, bottomLimit - height)),
		width,
		height,
		maxWidth,
		maxHeight,
		visible:
			maxWidth > 0 &&
			maxHeight > 0 &&
			anchor.left + anchor.width >= viewport.left &&
			anchor.left <= viewport.left + viewport.width &&
			anchor.top + anchor.height >= viewport.top &&
			anchor.top <= viewport.top + viewport.height,
	};
}
