import { type ReactNode, act } from "react";
import {
	type DomHarness,
	type Rendered,
	installDom,
} from "../../../settings/devices/testing/dom-harness";
import type { WorkbenchLayout } from "../contracts";

export {
	accessibleName,
	advance,
	allByRole,
	allByText,
	byRole,
	byText,
	click,
	clickByText,
	dropFiles,
	fire,
	inPortal,
	keyDown,
	queryByRole,
	settle,
	typeInto,
	type DomHarness,
	type Rendered,
} from "../../../settings/devices/testing/dom-harness";

/*
 * DOM tests of the workbench (PLAN §10). Call `installWorkbenchDom()` first, then import components
 * dynamically, then `mountWorkbench(node, { layout })`:
 *
 *   const dom = installWorkbenchDom();
 *   const { Dock } = await import("../dock/dock");
 *   afterEach(dom.cleanup);
 *   afterAll(dom.restore);
 *   const view = await mountWorkbench(<Dock {...props} variant="rail" />, { layout: DESKTOP_LAYOUT });
 *
 * The devices harness's ResizeObserver never fires and happy-dom answers `(pointer: fine)` for every
 * box, so `mountWorkbench` installs both from the layout: the interface root (`[data-fw-root]`) is
 * measured at the layout's size, and the pointer queries answer `touch` / `finePointer`.
 */

export interface BoxSize {
	readonly width: number;
	readonly height: number;
}

/** The size an observed element reports; null leaves it unmeasured (no callback). */
export type SizeOf = (
	target: Element,
	layout: WorkbenchLayout,
) => BoxSize | null;

const ROOT_SELECTOR = "[data-fw-root]";

/** The interface root gets the layout's box; other elements stay unmeasured. */
export const rootOnly: SizeOf = (target, layout) =>
	target.matches(ROOT_SELECTOR)
		? { width: layout.width, height: layout.height }
		: null;

let installed: DomHarness | null = null;

const saved = new Map<string, PropertyDescriptor | undefined>();

function setGlobal(target: object, key: string, value: unknown) {
	Object.defineProperty(target, key, {
		configurable: true,
		writable: true,
		value,
	});
}

function remember(key: string) {
	if (!saved.has(key))
		saved.set(key, Object.getOwnPropertyDescriptor(globalThis, key));
}

function restoreGlobals() {
	for (const [key, descriptor] of saved) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else delete (globalThis as Record<string, unknown>)[key];
	}
	saved.clear();
}

/** `installDom()` plus what the workbench needs: `CSS.escape` (the shell's focus executor). */
export function installWorkbenchDom(
	options: { readonly url?: string; readonly language?: string } = {},
): DomHarness {
	const dom = installDom(options);
	const win = dom.window as unknown as { CSS: unknown };
	remember("CSS");
	setGlobal(globalThis, "CSS", win.CSS);
	installed = {
		...dom,
		restore() {
			restoreGlobals();
			dom.restore();
			installed = null;
		},
	};
	return installed;
}

function rectOf(size: BoxSize) {
	return {
		x: 0,
		y: 0,
		top: 0,
		left: 0,
		width: size.width,
		height: size.height,
		right: size.width,
		bottom: size.height,
		toJSON: () => ({ ...size }),
	};
}

function entryOf(target: Element, size: BoxSize): ResizeObserverEntry {
	const box = [{ inlineSize: size.width, blockSize: size.height }];
	return {
		target,
		contentRect: rectOf(size),
		borderBoxSize: box,
		contentBoxSize: box,
		devicePixelContentBoxSize: box,
	} as unknown as ResizeObserverEntry;
}

interface Sizing {
	layout: WorkbenchLayout;
	sizeOf: SizeOf;
	readonly observers: Set<SizedObserver>;
}

class SizedObserver {
	readonly #callback: ResizeObserverCallback;
	readonly #sizing: Sizing;
	readonly #targets = new Set<Element>();

	constructor(callback: ResizeObserverCallback, sizing: Sizing) {
		this.#callback = callback;
		this.#sizing = sizing;
		sizing.observers.add(this);
	}

	observe(target: Element) {
		this.#targets.add(target);
		this.report([target]);
	}

	unobserve(target: Element) {
		this.#targets.delete(target);
	}

	disconnect() {
		this.#targets.clear();
		this.#sizing.observers.delete(this);
	}

	takeRecords() {
		return [];
	}

	/** Calls back with the current size of the given targets (all observed ones by default). */
	report(targets: readonly Element[] = [...this.#targets]) {
		const { layout, sizeOf } = this.#sizing;
		const entries: ResizeObserverEntry[] = [];
		for (const target of targets) {
			const size = sizeOf(target, layout);
			if (size) entries.push(entryOf(target, size));
		}
		if (entries.length > 0)
			this.#callback(entries, this as unknown as ResizeObserver);
	}
}

const POINTER_QUERIES: Readonly<
	Record<string, (layout: WorkbenchLayout) => boolean>
> = {
	"(pointer: coarse)": (layout) => layout.touch,
	"(pointer: fine)": (layout) => layout.finePointer,
	"(any-pointer: coarse)": (layout) => layout.touch,
	"(any-pointer: fine)": (layout) => layout.finePointer,
	"(hover: hover)": (layout) => layout.finePointer,
	"(hover: none)": (layout) => !layout.finePointer,
};

const normalizeQuery = (query: string) =>
	query
		.toLowerCase()
		.replace(/\s+/g, " ")
		.replace(/\(\s*/g, "(")
		.replace(/\s*\)/g, ")")
		.replace(/\s*:\s*/g, ": ")
		.trim();

function mediaList(query: string, matches: boolean): MediaQueryList {
	return {
		matches,
		media: query,
		onchange: null,
		addEventListener() {},
		removeEventListener() {},
		addListener() {},
		removeListener() {},
		dispatchEvent: () => false,
	} as MediaQueryList;
}

/** Pointer and hover queries answer from the layout; everything else goes to happy-dom (sized to the box). */
function layoutMedia(
	sizing: Sizing,
	fallback: (query: string) => MediaQueryList,
) {
	return (query: string) => {
		const answer = POINTER_QUERIES[normalizeQuery(query)];
		return answer ? mediaList(query, answer(sizing.layout)) : fallback(query);
	};
}

interface HappyWindow {
	matchMedia(query: string): MediaQueryList;
	happyDOM?: { setViewport(size: BoxSize): void };
}

function applySizing(dom: DomHarness, sizing: Sizing) {
	const win = dom.window as unknown as HappyWindow;
	win.happyDOM?.setViewport({
		width: sizing.layout.width,
		height: sizing.layout.height,
	});
	const fallback = Object.getPrototypeOf(win).matchMedia.bind(win);
	const media = layoutMedia(sizing, fallback);
	const Observer = class extends SizedObserver {
		constructor(callback: ResizeObserverCallback) {
			super(callback, sizing);
		}
	};
	for (const key of ["matchMedia", "ResizeObserver"]) remember(key);
	setGlobal(win, "matchMedia", media);
	setGlobal(globalThis, "matchMedia", media);
	setGlobal(win, "ResizeObserver", Observer);
	setGlobal(globalThis, "ResizeObserver", Observer);
}

export interface MountWorkbenchOptions {
	readonly layout: WorkbenchLayout;
	/** Defaults to the harness of the last `installWorkbenchDom()`. */
	readonly dom?: DomHarness;
	/** Which observed elements report a size; default `rootOnly`. */
	readonly sizeOf?: SizeOf;
}

export interface MountedWorkbench extends Rendered {
	readonly dom: DomHarness;
	/** The layout the box reports now. */
	layout(): WorkbenchLayout;
	/** Resize the box: every observer reports its elements again, inside `act`. */
	resize(layout: WorkbenchLayout): Promise<void>;
}

/**
 * Render a workbench view in a box of `layout`'s size: the interface root's ResizeObserver reports
 * that size and `(pointer: coarse)` / `(pointer: fine)` answer from the layout.
 */
export async function mountWorkbench(
	node: ReactNode,
	options: MountWorkbenchOptions,
): Promise<MountedWorkbench> {
	const dom = options.dom ?? installed;
	if (!dom)
		throw new Error(
			"mountWorkbench: call installWorkbenchDom() before importing components and mounting",
		);
	const sizing: Sizing = {
		layout: options.layout,
		sizeOf: options.sizeOf ?? rootOnly,
		observers: new Set(),
	};
	applySizing(dom, sizing);
	const view = await dom.render(node);
	return {
		...view,
		dom,
		layout: () => sizing.layout,
		async resize(layout) {
			sizing.layout = layout;
			dom.window.happyDOM.setViewport({
				width: layout.width,
				height: layout.height,
			});
			await act(async () => {
				for (const observer of sizing.observers) observer.report();
			});
		},
	};
}
