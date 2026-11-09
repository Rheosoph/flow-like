import { afterEach, expect, test } from "bun:test";
import {
	type HTMLButtonElement as HappyButton,
	Event as HappyEvent,
	Window,
} from "happy-dom";
import { act, useState } from "react";

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

async function render(
	node: (
		Picker: typeof import("./query-table-picker")["QueryTablePicker"],
	) => React.ReactNode,
) {
	const window = new Window();
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		MouseEvent: window.MouseEvent,
		CustomEvent: window.CustomEvent,
		Node: window.Node,
		NodeFilter: window.NodeFilter,
		navigator: window.navigator,
		window,
		Document: window.Document,
		MutationObserver: window.MutationObserver,
		ResizeObserver: window.ResizeObserver,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { createRoot } = await import("react-dom/client");
	const { QueryTablePicker } = await import("./query-table-picker");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	cleanup = async () => {
		await act(async () => root.unmount());
		await window.happyDOM.close();
	};
	await act(async () => root.render(node(QueryTablePicker)));
	return window.document.body;
}

test("search finds views and selecting keeps the exact table name", async () => {
	const selected: string[] = [];
	const container = await render((Picker) => {
		function Harness() {
			const [open, setOpen] = useState(true);
			return (
				<Picker
					tables={[
						{
							name: "orders",
							columns: [{ name: "id", type_name: "Utf8", position: 0 }],
						},
					]}
					views={[{ name: 'reviewed"orders' }]}
					onSelectTable={(name) => selected.push(name)}
					open={open}
					onOpenChange={setOpen}
				/>
			);
		}
		return <Harness />;
	});
	expect(container.textContent).toContain("1 column");
	const input = container.querySelector("input");
	if (!input) throw new Error("Missing table search");
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			HTMLInputElement.prototype,
			"value",
		)?.set?.call(input, "REVIEWED");
		input.dispatchEvent(new HappyEvent("input", { bubbles: true }));
	});
	expect(container.querySelector('[aria-label="Query orders"]')).toBeNull();
	const query = container.querySelector<HappyButton>(
		"[aria-label='Query reviewed\"orders']",
	);
	if (!query) throw new Error("Missing query action");
	await act(async () => query.click());
	expect(selected).toEqual(['reviewed"orders']);
	expect(container.querySelector('[role="dialog"]')).toBeNull();
});

test("an empty catalog explains how to find tables", async () => {
	const container = await render((Picker) => (
		<Picker tables={[]} onSelectTable={() => undefined} open />
	));
	expect(container.textContent).toContain("No tables available");
	expect(container.textContent).toContain("add a table in Sources");
	expect(container.querySelector("input")).toBeNull();
});
