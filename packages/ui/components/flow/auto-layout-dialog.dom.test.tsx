import { afterAll, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, useState } from "react";
import type { LayoutStyle } from "../../lib/flow-auto-layout";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	HTMLButtonElement: window.HTMLButtonElement,
	Element: window.Element,
	Node: window.Node,
	NodeFilter: window.NodeFilter,
	MutationObserver: window.MutationObserver,
	CustomEvent: window.CustomEvent,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(0), 0),
	cancelAnimationFrame: (id: number) => clearTimeout(id),
	IS_REACT_ACT_ENVIRONMENT: true,
});

const translate = (
	_key: string,
	fallback: string,
	variables: Record<string, unknown> = {},
) =>
	fallback.replace(/\{\{(\w+)\}\}/g, (_, key) => String(variables[key] ?? ""));
mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({ t: translate }),
}));

const { createRoot } = await import("react-dom/client");
const { AutoLayoutDialog } = await import("./auto-layout-dialog");

afterAll(() => mock.restore());

for (const [label, expected] of [
	["Compact", "compact"],
	["Routed", "routed"],
	["Expanded", "expanded"],
] as const) {
	test(`${label} applies only after selection and closes the dialog`, async () => {
		const selected: LayoutStyle[] = [];
		function Harness() {
			const [open, setOpen] = useState(true);
			return (
				<AutoLayoutDialog
					open={open}
					onOpenChange={setOpen}
					onSelect={(style) => selected.push(style)}
					selectionCount={3}
				/>
			);
		}
		const container = window.document.createElement("div");
		window.document.body.appendChild(container);
		const root = createRoot(container as unknown as HTMLElement);
		try {
			await act(async () => root.render(<Harness />));
			const dialog = window.document.querySelector('[role="dialog"]');
			expect(dialog).not.toBeNull();
			expect(selected).toEqual([]);
			expect(dialog?.textContent).not.toContain("Balanced");
			expect(dialog?.textContent).toContain("3 selected nodes");
			const button = Array.from(dialog?.querySelectorAll("button") ?? []).find(
				(candidate) => candidate.querySelector("span")?.textContent === label,
			);
			expect(button).toBeDefined();
			await act(async () => button?.click());
			expect(selected).toEqual([expected]);
			expect(window.document.querySelector('[role="dialog"]')).toBeNull();
		} finally {
			await act(async () => root.unmount());
			container.remove();
		}
	});
}
