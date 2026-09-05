import { afterEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { mutationListeners } from "happy-dom/lib/PropertySymbol.js";
import { compile } from "tailwindcss";
import {
	observeRuntimeTailwind,
	runtimeTailwindTheme,
} from "./runtime-tailwind";

const theme: Array<[string, string]> = [
	["--spacing", ".25rem"],
	["--breakpoint-md", "48rem"],
	["--color-fuchsia-950", "oklch(29.3% 0.136 325.661)"],
	["--color-primary", "#f60"],
	["--primary", "#f60"],
	["--radius", ".375rem"],
];

const windows: Window[] = [];
const cleanups: Array<() => void> = [];
const retainedCallbacks: unknown[] = [];

function documentWithRoot(classes: string) {
	const browserWindow = new Window();
	Object.assign(browserWindow, { SyntaxError, TypeError });
	// happy-dom keeps its observer callback only in a WeakRef, so Bun can collect
	// it between turns. Retain that callback while testing real DOM mutations.
	const Observer = browserWindow.MutationObserver;
	Object.defineProperty(browserWindow, "MutationObserver", {
		value: class extends Observer {
			observe(...args: Parameters<InstanceType<typeof Observer>["observe"]>) {
				super.observe(...args);
				for (const listener of args[0][mutationListeners]) {
					retainedCallbacks.push(listener.callback.deref());
				}
			}
		},
	});
	windows.push(browserWindow);
	const doc = browserWindow.document as unknown as Document;
	for (const [name, value] of theme) {
		doc.documentElement.style.setProperty(name, value);
	}
	const root = doc.createElement("div");
	root.className = classes;
	doc.body.appendChild(root);
	cleanups.push(observeRuntimeTailwind(root));
	return { doc, root };
}

async function compiledCss(doc: Document, expected: string): Promise<string> {
	for (let attempt = 0; attempt < 50; attempt++) {
		const css = doc.head.querySelector(
			"[data-a2ui-runtime-tailwind]",
		)?.textContent;
		if (css?.includes(expected)) return css;
		await new Promise((resolve) => setTimeout(resolve, 5));
	}
	throw new Error(`Runtime stylesheet never contained ${expected}`);
}

afterEach(() => {
	for (const cleanup of cleanups.splice(0)) cleanup();
	for (const browserWindow of windows.splice(0)) browserWindow.close();
	retainedCallbacks.length = 0;
});

describe("runtime Tailwind", () => {
	test("compiles arbitrary values, responsive variants, and uncommon theme colors", async () => {
		const compiler = await compile(runtimeTailwindTheme(theme));
		const css = compiler.build([
			"w-[137px]",
			"md:gap-13",
			"bg-fuchsia-950",
			"hover:bg-primary/35",
			"dark:text-primary",
			"rounded-lg",
			"[&>span]:p-7.5",
		]);
		expect(css).toContain("width: 137px");
		expect(css).toContain("@media (width >= 48rem)");
		expect(css).toContain("var(--color-fuchsia-950");
		expect(css).toContain("color-mix(in oklab, var(--primary) 35%");
		expect(css).toContain("&:is(.dark *)");
		expect(css).toContain("border-radius: var(--radius)");
		expect(css).toContain("&>span");
		expect(css).not.toContain(":root");
		expect(css).not.toContain("box-sizing");
	});

	test("the application build retains unused theme tokens and animation keyframes", async () => {
		const { compile: compileStylesheet } = await import("@tailwindcss/node");
		const input = await Bun.file(
			new URL("../global.css", import.meta.url),
		).text();
		const compiler = await compileStylesheet(input, {
			base: new URL("../", import.meta.url).pathname,
			onDependency: () => {},
		});
		const css = compiler.build([]);
		expect(css).toContain("--color-fuchsia-950:");
		expect(css).toContain("--color-primary:");
		expect(css).toContain("--breakpoint-md:");
		expect(css).toContain("--animate-ai-bounce:");
		expect(css).toContain("@keyframes spin");
	});

	test("observes class edits and asynchronously inserted widget children", async () => {
		const { doc, root } = documentWithRoot("w-[137px]");
		await compiledCss(doc, "width: 137px");
		root.className = "md:gap-13";
		await compiledCss(doc, "@media (width >= 48rem)");
		const widget = doc.createElement("section");
		widget.innerHTML = '<span class="bg-fuchsia-950"></span>';
		root.appendChild(widget);
		await compiledCss(doc, "var(--color-fuchsia-950");
		expect(root.children.length).toBe(1);
		expect(root.querySelector("style")).toBeNull();
	});

	test("waits for theme styles loaded after the surface has mounted", async () => {
		const { doc, root } = documentWithRoot("bg-fuchsia-950");
		doc.documentElement.removeAttribute("style");
		await new Promise((resolve) => setTimeout(resolve, 10));
		expect(doc.head.querySelector("[data-a2ui-runtime-tailwind]")).toBeNull();
		const styles = doc.createElement("style");
		styles.textContent = `:root { ${theme.map(([name, value]) => `${name}: ${value};`).join(" ")} }`;
		doc.head.appendChild(styles);
		await compiledCss(doc, "var(--color-fuchsia-950");
		expect(root.children.length).toBe(0);
	});

	test("keeps the sheet connected when a root is replaced in the same render", async () => {
		const { doc, root } = documentWithRoot("w-[137px]");
		await compiledCss(doc, "width: 137px");
		const sheet = doc.head.querySelector("[data-a2ui-runtime-tailwind]");
		cleanups.pop()?.();
		cleanups.push(observeRuntimeTailwind(root));
		expect(sheet?.isConnected).toBe(true);
		await Promise.resolve();
		expect(sheet?.isConnected).toBe(true);
	});

	test("keeps styles in each owner document and shares a sheet between surface roots", async () => {
		const first = documentWithRoot("w-[137px]");
		const second = documentWithRoot("h-[219px]");
		const sibling = first.doc.createElement("div");
		sibling.className = "p-7.5";
		first.doc.body.appendChild(sibling);
		const detach = observeRuntimeTailwind(sibling);
		const firstCss = await compiledCss(first.doc, "* 7.5)");
		const secondCss = await compiledCss(second.doc, "height: 219px");
		expect(firstCss).not.toContain("height: 219px");
		expect(secondCss).not.toContain("width: 137px");
		expect(first.doc.head.querySelectorAll("style").length).toBe(1);
		detach();
		expect(first.doc.head.querySelectorAll("style").length).toBe(1);
	});

	test("removes the last sheet and stops compiling after the surface unmounts", async () => {
		const { doc, root } = documentWithRoot("w-[137px]");
		await compiledCss(doc, "width: 137px");
		cleanups.pop()?.();
		root.className = "h-[219px]";
		await new Promise((resolve) => setTimeout(resolve, 10));
		expect(doc.head.querySelector("[data-a2ui-runtime-tailwind]")).toBeNull();
	});
});
