import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";

const inlineWidgetDef = {
	name: "Artikel",
	rootComponentId: "card",
	components: [
		{
			id: "card",
			component: { type: "column", children: { explicitList: ["badges"] } },
		},
		{
			id: "badges",
			component: { type: "row", children: { explicitList: ["badge-1"] } },
		},
	],
};

async function renderInstance(
	renderChild: (childId: string) => React.ReactNode,
) {
	const window = new Window({ url: "https://local/use" });
	Object.assign(globalThis, {
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		window,
		IS_REACT_ACT_ENVIRONMENT: true,
	});

	const [
		{ A2UIWidgetInstance },
		{ useBackendStore },
		{ QueryClient, QueryClientProvider },
	] = await Promise.all([
		import("./A2UIWidgetInstance"),
		import("../../../state/backend-state"),
		import("@tanstack/react-query"),
	]);

	// An inline widget definition never fetches, but the node reads
	// `backend.widgetState` while wiring the query up.
	useBackendStore.getState().setBackend({
		widgetState: { getWidget: async () => undefined },
	} as never);

	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	const root = createRoot(host as unknown as HTMLElement);

	await act(() => {
		root.render(
			createElement(
				QueryClientProvider as never,
				{ client: new QueryClient() } as never,
				createElement(
					A2UIWidgetInstance as never,
					{
						component: {
							type: "widgetInstance",
							instanceId: "inst-1",
							widgetId: "artikel",
							inlineWidgetDef,
						},
						componentId: "inst-1",
						surfaceId: "page-1",
						renderChild,
					} as never,
				),
			),
		);
	});

	return host as unknown as HTMLElement;
}

describe("A2UIWidgetInstance children pushed in at runtime", () => {
	test("renders a surface element pushed into a widget-internal container", async () => {
		const requested: string[] = [];
		const host = await renderInstance((childId) => {
			requested.push(childId);
			return createElement("div", { "data-external": childId }, "badge");
		});

		expect(requested).toEqual(["badge-1"]);
		expect(host.innerHTML).toContain('data-external="badge-1"');
	});

	test("keeps rendering nothing when the surface has no such element", async () => {
		const host = await renderInstance(() => null);
		expect(host.innerHTML).not.toContain("data-external");
	});
});
