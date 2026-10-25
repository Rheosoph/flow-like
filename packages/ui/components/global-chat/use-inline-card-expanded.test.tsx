import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { useInlineCardExpanded } from "./use-inline-card-expanded";

let latest: ReturnType<typeof useInlineCardExpanded> | null = null;

function Probe({ id }: { id: string }) {
	latest = useInlineCardExpanded(id, true);
	return null;
}

async function setup() {
	const window = new Window();
	Object.assign(globalThis, {
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		window,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { createRoot } = await import("react-dom/client");
	const container = document.createElement("div");
	document.body.append(container);
	return createRoot(container);
}

describe("useInlineCardExpanded", () => {
	test("a remounted card keeps its expanded state and skips its entrance", async () => {
		const first = await setup();
		await act(async () => first.render(<Probe id="card-1" />));
		expect(latest).toMatchObject({ expanded: true, remounted: false });

		await act(async () => latest?.setExpanded(false));
		await act(async () => first.unmount());

		const second = await setup();
		await act(async () => second.render(<Probe id="card-1" />));
		expect(latest).toMatchObject({ expanded: false, remounted: true });
		await act(async () => second.unmount());

		const third = await setup();
		await act(async () => third.render(<Probe id="card-2" />));
		expect(latest).toMatchObject({ expanded: true, remounted: false });
		await act(async () => third.unmount());
	});
});
