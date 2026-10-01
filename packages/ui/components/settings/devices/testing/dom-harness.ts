import { createI18n } from "@flow-like/locales";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";

/*
 * DOM test harness for the devices area (plan §4.0.2).
 *
 * Call `installDom()` before importing `react-dom/client` or any component:
 * react-dom probes the DOM once at load (memory happy-dom-react-dom-import-order).
 * Components are therefore imported dynamically after it:
 *
 *   const dom = installDom();
 *   const { StatusChip } = await import("../primitives/status-chip");
 *   afterEach(dom.cleanup);
 *   afterAll(dom.restore);
 *
 * Never `mock.module` the device lib, backend state or `@flow-like/locales`.
 */

const GLOBAL_KEYS = [
	"window",
	"document",
	"navigator",
	"localStorage",
	"sessionStorage",
	"location",
	"HTMLElement",
	"HTMLInputElement",
	"HTMLTextAreaElement",
	"HTMLSelectElement",
	"HTMLButtonElement",
	"HTMLAnchorElement",
	"HTMLIFrameElement",
	"SVGElement",
	"Element",
	"Node",
	"Text",
	"DocumentFragment",
	"MutationObserver",
	"NodeFilter",
	"Event",
	"CustomEvent",
	"InputEvent",
	"FocusEvent",
	"KeyboardEvent",
	"MouseEvent",
	"PointerEvent",
	"File",
	"FileList",
	"DataTransfer",
	"getComputedStyle",
	"matchMedia",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"ResizeObserver",
	"IntersectionObserver",
	"DOMRect",
	"IS_REACT_ACT_ENVIRONMENT",
] as const;

class InertObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
	takeRecords() {
		return [];
	}
}

export interface Rendered {
	container: HTMLElement;
	rerender(ui: ReactNode): Promise<void>;
	unmount(): Promise<void>;
}

export interface DomHarness {
	window: Window;
	document: Document;
	/** Texts written through `navigator.clipboard.writeText`. */
	clipboard: string[];
	render(ui: ReactNode): Promise<Rendered>;
	/** Unmount everything rendered and empty `document.body`. */
	cleanup(): Promise<void>;
	/** Put the globals back (call in `afterAll`). */
	restore(): void;
	/** Re-assign the globals (call in `beforeAll` when several DOM files share a process). */
	reinstall(): void;
}

const ELEMENT_POLYFILLS: Record<string, () => unknown> = {
	hasPointerCapture: () => false,
	setPointerCapture: () => {},
	releasePointerCapture: () => {},
	scrollIntoView: () => {},
};

function polyfillElement(win: Window) {
	const proto = win.Element.prototype as unknown as Record<string, unknown>;
	for (const [key, fn] of Object.entries(ELEMENT_POLYFILLS)) proto[key] ??= fn;
}

export function installDom(
	options: { url?: string; language?: string } = {},
): DomHarness {
	const win = new Window({ url: options.url ?? "https://app.test" });
	Object.assign(win, { SyntaxError, TypeError, Error });
	polyfillElement(win);

	const clipboard: string[] = [];
	Object.defineProperty(win.navigator, "clipboard", {
		configurable: true,
		value: {
			writeText: async (text: string) => {
				clipboard.push(text);
			},
			readText: async () => clipboard.at(-1) ?? "",
		},
	});

	const saved = GLOBAL_KEYS.map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	const values: Record<(typeof GLOBAL_KEYS)[number], unknown> = {
		window: win,
		document: win.document,
		navigator: win.navigator,
		localStorage: win.localStorage,
		sessionStorage: win.sessionStorage,
		location: win.location,
		HTMLElement: win.HTMLElement,
		HTMLInputElement: win.HTMLInputElement,
		HTMLTextAreaElement: win.HTMLTextAreaElement,
		HTMLSelectElement: win.HTMLSelectElement,
		HTMLButtonElement: win.HTMLButtonElement,
		HTMLAnchorElement: win.HTMLAnchorElement,
		HTMLIFrameElement: win.HTMLIFrameElement,
		SVGElement: win.SVGElement,
		Element: win.Element,
		Node: win.Node,
		Text: win.Text,
		DocumentFragment: win.DocumentFragment,
		MutationObserver: win.MutationObserver,
		NodeFilter: win.NodeFilter,
		Event: win.Event,
		CustomEvent: win.CustomEvent,
		InputEvent: win.InputEvent,
		FocusEvent: win.FocusEvent,
		KeyboardEvent: win.KeyboardEvent,
		MouseEvent: win.MouseEvent,
		PointerEvent: win.PointerEvent,
		File: win.File,
		FileList: win.FileList,
		DataTransfer: win.DataTransfer,
		getComputedStyle: win.getComputedStyle.bind(win),
		matchMedia: win.matchMedia.bind(win),
		requestAnimationFrame: (cb: FrameRequestCallback) =>
			setTimeout(() => cb(Date.now()), 0),
		cancelAnimationFrame: (id: ReturnType<typeof setTimeout>) =>
			clearTimeout(id),
		ResizeObserver: InertObserver,
		IntersectionObserver: InertObserver,
		DOMRect: win.DOMRect,
		IS_REACT_ACT_ENVIRONMENT: true,
	};

	const install = () => {
		for (const key of GLOBAL_KEYS) {
			Object.defineProperty(globalThis, key, {
				configurable: true,
				writable: true,
				value: values[key],
			});
		}
	};
	install();
	createI18n({ language: options.language ?? "en" });

	const mounted = new Set<{ unmount(): void; container: HTMLElement }>();

	return {
		window: win,
		document: win.document as unknown as Document,
		clipboard,
		async render(ui) {
			const { createRoot } = await import("react-dom/client");
			const container = win.document.createElement(
				"div",
			) as unknown as HTMLElement;
			(win.document.body as unknown as HTMLElement).append(container);
			const root = createRoot(container);
			const entry = { unmount: () => root.unmount(), container };
			mounted.add(entry);
			await act(async () => root.render(ui));
			return {
				container,
				rerender: async (next) => {
					await act(async () => root.render(next));
				},
				unmount: async () => {
					await act(async () => root.unmount());
					mounted.delete(entry);
					container.remove();
				},
			};
		},
		async cleanup() {
			for (const entry of mounted) {
				await act(async () => entry.unmount());
				entry.container.remove();
			}
			mounted.clear();
			win.document.body.innerHTML = "";
			clipboard.length = 0;
		},
		restore() {
			for (const [key, descriptor] of saved) {
				if (descriptor) Object.defineProperty(globalThis, key, descriptor);
				else delete (globalThis as Record<string, unknown>)[key];
			}
		},
		reinstall: install,
	};
}

/* Queries. Every query defaults to `document.body`, so portalled content is found too. */

type Matcher = string | RegExp;

const normalize = (text: string | null | undefined) =>
	(text ?? "").replace(/\s+/g, " ").trim();

function matches(text: string, matcher: Matcher) {
	return typeof matcher === "string"
		? normalize(text) === normalize(matcher)
		: matcher.test(normalize(text));
}

function body(): HTMLElement {
	return document.body;
}

const IMPLICIT_ROLE: Record<string, string> = {
	button: "button",
	"a[href]": "link",
	"h1,h2,h3,h4,h5,h6": "heading",
	table: "table",
	tr: "row",
	td: "cell",
	th: "columnheader",
	'input[type="checkbox"]': "checkbox",
	'input[type="radio"]': "radio",
	'input:not([type]),input[type="text"],input[type="password"],input[type="search"],input[type="email"],input[type="url"],input[type="number"],textarea':
		"textbox",
	select: "combobox",
	dialog: "dialog",
	nav: "navigation",
	ul: "list",
	ol: "list",
	li: "listitem",
	dl: "list",
	fieldset: "group",
	"section[aria-label],section[aria-labelledby]": "region",
};

function roleOf(el: Element) {
	const explicit = el.getAttribute("role");
	if (explicit) return explicit.split(" ")[0] ?? null;
	for (const [selector, role] of Object.entries(IMPLICIT_ROLE)) {
		if (el.matches(selector)) return role;
	}
	return null;
}

function labelledByText(el: Element) {
	const ids = el.getAttribute("aria-labelledby");
	if (!ids) return null;
	return ids
		.split(/\s+/)
		.map((id) => el.ownerDocument.getElementById(id)?.textContent ?? "")
		.join(" ");
}

function labelElementText(el: Element) {
	const forLabel = el.id
		? el.ownerDocument.querySelector(`label[for="${el.id}"]`)
		: null;
	if (forLabel) return forLabel.textContent;
	const wrapping = el.closest("label");
	return wrapping && el.matches("input,select,textarea")
		? wrapping.textContent
		: null;
}

const NAME_SOURCES: readonly ((el: Element) => string | null)[] = [
	labelledByText,
	(el) => el.getAttribute("aria-label"),
	labelElementText,
	(el) => el.getAttribute("alt"),
];

/** The accessible name, close enough for tests: aria-labelledby, aria-label, `<label for>`, alt, text, title. */
export function accessibleName(el: Element): string {
	for (const source of NAME_SOURCES) {
		const name = normalize(source(el));
		if (name) return name;
	}
	return normalize(el.textContent) || normalize(el.getAttribute("title"));
}

export function allByRole(
	role: string,
	name?: Matcher,
	root: ParentNode = body(),
): HTMLElement[] {
	return Array.from(root.querySelectorAll<HTMLElement>("*")).filter(
		(el) =>
			roleOf(el) === role &&
			!el.closest("[aria-hidden='true']") &&
			(name === undefined || matches(accessibleName(el), name)),
	);
}

export function queryByRole(
	role: string,
	name?: Matcher,
	root?: ParentNode,
): HTMLElement | null {
	return allByRole(role, name, root)[0] ?? null;
}

export function byRole(
	role: string,
	name?: Matcher,
	root?: ParentNode,
): HTMLElement {
	const found = queryByRole(role, name, root);
	if (!found) {
		const seen = allByRole(role, undefined, root)
			.map((el) => JSON.stringify(accessibleName(el)))
			.join(", ");
		throw new Error(
			`No element with role "${role}"${name === undefined ? "" : ` named ${String(name)}`}. Seen: ${seen || "none"}`,
		);
	}
	return found;
}

/** The innermost elements whose own text matches (ancestors that merely contain it are skipped). */
export function allByText(
	text: Matcher,
	root: ParentNode = body(),
): HTMLElement[] {
	const hits = Array.from(root.querySelectorAll<HTMLElement>("*")).filter(
		(el) => matches(el.textContent ?? "", text),
	);
	return hits.filter(
		(el) => !hits.some((other) => other !== el && el.contains(other)),
	);
}

export function byText(text: Matcher, root?: ParentNode): HTMLElement {
	const found = allByText(text, root)[0];
	if (!found) throw new Error(`No element with text ${String(text)}`);
	return found;
}

/** The newest open portal layer (dialog, alertdialog, menu, listbox), or one with the given role. */
export function inPortal(role?: string): HTMLElement {
	const roles = role ? [role] : ["alertdialog", "dialog", "menu", "listbox"];
	const layers = roles.flatMap((r) => allByRole(r));
	const last = layers.at(-1);
	if (!last) throw new Error(`No open portal layer (${roles.join(", ")})`);
	return last;
}

/* Interaction. Each step runs inside `act` so effects and state updates flush. */

/** Dispatch any event (drag, drop, custom) inside `act`. */
export async function fire(el: Element, event: Event): Promise<void> {
	await act(async () => {
		el.dispatchEvent(event);
	});
}

/** Drop files on an element, as a user dragging them in from the desktop. */
export async function dropFiles(el: Element, files: File[]): Promise<void> {
	const view = el.ownerDocument.defaultView as unknown as typeof globalThis;
	const drop = new view.Event("drop", { bubbles: true, cancelable: true });
	Object.defineProperty(drop, "dataTransfer", {
		value: { files, types: ["Files"] },
	});
	await fire(el, drop);
}

export async function click(el: Element): Promise<void> {
	await act(async () => {
		const view = el.ownerDocument.defaultView as unknown as typeof globalThis;
		el.dispatchEvent(
			new view.PointerEvent("pointerdown", { bubbles: true, button: 0 }),
		);
		el.dispatchEvent(
			new view.MouseEvent("mousedown", { bubbles: true, button: 0 }),
		);
		el.dispatchEvent(
			new view.PointerEvent("pointerup", { bubbles: true, button: 0 }),
		);
		el.dispatchEvent(
			new view.MouseEvent("mouseup", { bubbles: true, button: 0 }),
		);
		if (el instanceof view.HTMLElement) el.click();
		else
			el.dispatchEvent(
				new view.MouseEvent("click", { bubbles: true, button: 0 }),
			);
	});
}

/** Click the clickable element (button, link, role) whose text matches, or the innermost text match. */
export async function clickByText(
	text: Matcher,
	root?: ParentNode,
): Promise<HTMLElement> {
	const hit = byText(text, root);
	const target =
		hit.closest<HTMLElement>(
			"button,a,[role=button],[role=menuitem],[role=tab],[role=option],label",
		) ?? hit;
	await click(target);
	return target;
}

/** Set an input's value the way a user would, so React's onChange runs. */
export async function typeInto(el: Element, value: string): Promise<void> {
	await act(async () => {
		const view = el.ownerDocument.defaultView as unknown as typeof globalThis;
		const proto =
			el instanceof view.HTMLTextAreaElement
				? view.HTMLTextAreaElement.prototype
				: el instanceof view.HTMLSelectElement
					? view.HTMLSelectElement.prototype
					: view.HTMLInputElement.prototype;
		Object.getOwnPropertyDescriptor(proto, "value")?.set?.call(el, value);
		el.dispatchEvent(new view.Event("input", { bubbles: true }));
		el.dispatchEvent(new view.Event("change", { bubbles: true }));
	});
}

export async function keyDown(
	el: Element,
	key: string,
	init: KeyboardEventInit = {},
): Promise<void> {
	await act(async () => {
		const view = el.ownerDocument.defaultView as unknown as typeof globalThis;
		el.dispatchEvent(
			new view.KeyboardEvent("keydown", { key, bubbles: true, ...init }),
		);
		el.dispatchEvent(
			new view.KeyboardEvent("keyup", { key, bubbles: true, ...init }),
		);
	});
}

/** Let real time pass (countdowns, debounces) inside `act`. */
export async function advance(ms: number): Promise<void> {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

/** Flush pending microtasks, effects and one macrotask. */
export async function settle(): Promise<void> {
	await act(async () => {
		await Promise.resolve();
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}
