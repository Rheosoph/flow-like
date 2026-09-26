import { afterEach, expect, test } from "bun:test";
import { gzipSync } from "node:zlib";
import { Window } from "happy-dom";
import { act } from "react";

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

const FEATURE = { hex: "406544", category: "A3", altitude: 37000 };
const gzippedFeature = () =>
	Array.from(gzipSync(new TextEncoder().encode(JSON.stringify(FEATURE))));

async function render(
	node: (components: typeof import("./binary-value-cell")) => React.ReactNode,
) {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
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
	const components = await import("./binary-value-cell");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	cleanup = async () => {
		await act(async () => root.unmount());
		await window.happyDOM.close();
	};
	await act(async () => root.render(node(components)));
	return container;
}

async function waitFor(check: () => boolean) {
	for (let attempt = 0; attempt < 100; attempt++) {
		if (check()) return;
		await act(async () => new Promise((resolve) => setTimeout(resolve, 10)));
	}
	throw new Error("condition never held");
}

test("a gzipped JSON cell reads as the JSON inside it", async () => {
	const container = await render(({ BinaryCellPreview }) => (
		<BinaryCellPreview value={gzippedFeature()} onClick={() => undefined} />
	));
	await waitFor(() => container.textContent?.includes("406544") ?? false);
	expect(container.textContent).toContain("gzip");
	expect(container.textContent).toContain('{"hex":"406544"');
	expect(container.textContent).not.toContain("31,139");
});

test("the detail view pretty-prints the JSON and can switch to the stored bytes", async () => {
	const container = await render(({ BinaryValueDetail }) => (
		<BinaryValueDetail value={gzippedFeature()} />
	));
	await waitFor(() => container.querySelector("pre") !== null);
	expect(container.querySelector("pre")?.textContent).toBe(
		JSON.stringify(FEATURE, null, 2),
	);

	const rawBytes = Array.from(container.querySelectorAll("button")).find(
		(button) => button.textContent === "Raw bytes",
	);
	await act(async () => rawBytes?.click());
	expect(container.querySelector("pre")?.textContent).toStartWith(
		"00000000  1f 8b 08",
	);
});

test("bytes that are not text stay a hex dump with their format", async () => {
	const png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
	const container = await render(({ BinaryValueDetail }) => (
		<BinaryValueDetail value={png} />
	));
	await waitFor(() => container.querySelector("pre") !== null);
	expect(container.textContent).toContain("PNG");
	expect(container.querySelector("pre")?.textContent).toStartWith(
		"00000000  89 50 4e 47",
	);
});
