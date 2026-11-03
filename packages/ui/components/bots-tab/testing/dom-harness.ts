import botsLocale from "@flow-like/locales/locales/en/bots.json";
import { Window } from "happy-dom";
import { act } from "react";

/**
 * Shared DOM setup for the Bots tab's bun DOM tests. Importing it installs the globals, so it
 * must come before anything that loads react-dom; tests import react-dom dynamically after it.
 */
export const domWindow = new Window({
	url: "https://localhost/chat",
	// Still avatars: their blink and hop timers would otherwise land between acts.
	settings: { device: { prefersReducedMotion: "reduce" } },
});
Object.assign(domWindow, { SyntaxError, TypeError, Error });

const WINDOW_GLOBALS = [
	"document",
	"navigator",
	"localStorage",
	"sessionStorage",
	"HTMLElement",
	"HTMLTextAreaElement",
	"HTMLInputElement",
	"HTMLButtonElement",
	"Element",
	"Node",
	"Text",
	"DocumentFragment",
	"SVGElement",
	"MutationObserver",
	"NodeFilter",
	"Event",
	"CustomEvent",
	"InputEvent",
	"FocusEvent",
	"KeyboardEvent",
	"MouseEvent",
	"PointerEvent",
] as const;

const STUBBED_GLOBALS = [
	"window",
	"getComputedStyle",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"ResizeObserver",
	"IntersectionObserver",
	"matchMedia",
	"IS_REACT_ACT_ENVIRONMENT",
] as const;

const previous = [...WINDOW_GLOBALS, ...STUBBED_GLOBALS].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);

class InertObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
}

export function installDomGlobals() {
	const source = domWindow as unknown as Record<string, unknown>;
	const globals: Record<string, unknown> = {
		window: domWindow,
		getComputedStyle: domWindow.getComputedStyle.bind(domWindow),
		requestAnimationFrame: (callback: FrameRequestCallback) =>
			setTimeout(() => callback(0), 0),
		cancelAnimationFrame: (id: number) => clearTimeout(id),
		ResizeObserver: InertObserver,
		IntersectionObserver: InertObserver,
		matchMedia: () => ({
			matches: false,
			addEventListener() {},
			removeEventListener() {},
		}),
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	for (const key of WINDOW_GLOBALS) globals[key] = source[key];
	Object.assign(globalThis, globals);
}

export function restoreDomGlobals() {
	for (const [key, descriptor] of previous) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
}

installDomGlobals();

function fill(template: string, options: Record<string, unknown>) {
	return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
		String(options[name] ?? ""),
	);
}

function english(key: string) {
	const value = key
		.replace(/^bots:/, "")
		.split(".")
		.reduce<unknown>(
			(node, part) => (node as Record<string, unknown> | undefined)?.[part],
			botsLocale,
		);
	return typeof value === "string" ? value : key;
}

function optionsOf(args: readonly unknown[]) {
	const record = args.find(
		(arg): arg is Record<string, unknown> =>
			typeof arg === "object" && arg !== null,
	);
	return record ?? {};
}

/** `t()` with the inline English defaults, or the source file's text when a call has none. */
export function translate(key: string, ...args: unknown[]) {
	const options = optionsOf(args);
	const candidates = [
		options.count === 1 ? options.defaultValue_one : undefined,
		args[0],
		options.defaultValue,
		english(key),
	];
	const template = candidates.find((value) => typeof value === "string");
	return fill(String(template), options);
}

export const fakeI18n = { t: translate, language: "en", on() {}, off() {} };

/** Lets queries, batched lookups and timers land inside act. */
export async function settle(ms = 30) {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}
