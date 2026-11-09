import { afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, useState } from "react";

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

async function render(
	node: (components: typeof import("./result-value")) => React.ReactNode,
) {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
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
		Text: window.Text,
		MutationObserver: window.MutationObserver,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { createRoot } = await import("react-dom/client");
	const components = await import("./result-value");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	cleanup = async () => {
		await act(async () => root.unmount());
		await window.happyDOM.close();
	};
	await act(async () => root.render(node(components)));
	return window.document.body;
}

const clickButton = async (
	container: { querySelectorAll: Element["querySelectorAll"] },
	label: string,
) => {
	const button = Array.from(container.querySelectorAll("button")).find(
		(candidate) =>
			candidate.getAttribute("aria-label") === label ||
			candidate.textContent === label,
	);
	if (!button) throw new Error(`no ${label} button`);
	await act(async () => (button as unknown as HTMLButtonElement).click());
};

test("query text opens a readable Markdown preview and preserves the full source", async () => {
	const text = `### Joining process\n\n**Pressure** and tooling details.\n\n${"Long paragraph. ".repeat(100)}`;
	const container = await render(({ ResultCellValue }) => {
		function PreviewHarness() {
			const [open, setOpen] = useState(false);
			return (
				<>
					<ResultCellValue
						name="content"
						kind="text"
						value={text}
						onInspect={() => setOpen(true)}
					/>
					{open && <Detail />}
				</>
			);
		}
		return <PreviewHarness />;
	});

	const { CellInspectorDialog } = await import("./cell-inspector-dialog");
	function Detail() {
		return (
			<CellInspectorDialog
				cell={{
					column: { name: "content", type_name: "Utf8", position: 0 },
					kind: "text",
					value: text,
				}}
				onOpenChange={() => undefined}
			/>
		);
	}

	await clickButton(container, "Preview content");
	expect(container.querySelector("[role=dialog]")).not.toBeNull();
	expect(container.querySelector("h3")?.textContent).toBe("Joining process");
	expect(container.querySelector("strong")?.textContent).toBe("Pressure");
	await clickButton(container, "Raw");
	expect(container.querySelector("pre")?.textContent).toBe(text);
	await clickButton(container, "Copy");
	expect(await navigator.clipboard.readText()).toBe(text);
});

test("query JSON details expose nested structure and exact raw content", async () => {
	const value = { state: "ready", payload: JSON.stringify({ count: 3 }) };
	const container = await render(({ ResultDetailValue }) => (
		<ResultDetailValue name="payload" kind="json" value={value} />
	));
	expect(container.textContent).toContain("JSON object · 2 keys");
	expect(container.textContent).toContain("JSON string");
	expect(container.textContent).toContain("count: 3");
	await clickButton(container, "Raw");
	expect(container.querySelector("pre")?.textContent).toBe(
		JSON.stringify(value),
	);
});

test("empty strings remain distinguishable from SQL null", async () => {
	const container = await render(({ ResultCellValue }) => (
		<>
			<ResultCellValue name="empty" kind="text" value="" />
			<ResultCellValue name="missing" kind="text" value={null} />
		</>
	));
	expect(container.textContent).toContain("Empty string");
	expect(container.textContent).toContain("NULL");
});
