import { afterEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { ApiResponseError } from "../../../lib/api-error";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
	for (const cleanup of cleanups.splice(0)) await cleanup();
});

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
	fetchError?: Error,
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

	const previousBackend = useBackendStore.getState().backend;
	useBackendStore.getState().setBackend({
		widgetState: {
			getWidget: async () => {
				if (fetchError) throw fetchError;
				return undefined;
			},
		},
	} as never);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	if (fetchError) {
		client.setQueryData(["getWidget", "app-1", "artikel"], inlineWidgetDef);
	}

	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	const root = createRoot(host as unknown as HTMLElement);
	cleanups.push(async () => {
		await act(() => root.unmount());
		client.clear();
		useBackendStore.setState({ backend: previousBackend });
		await window.happyDOM.abort();
	});

	await act(() => {
		root.render(
			createElement(
				QueryClientProvider as never,
				{ client } as never,
				createElement(
					A2UIWidgetInstance as never,
					{
						component: {
							type: "widgetInstance",
							instanceId: "inst-1",
							widgetId: "artikel",
							inlineWidgetDef: fetchError ? undefined : inlineWidgetDef,
						},
						appId: "app-1",
						componentId: "inst-1",
						surfaceId: "page-1",
						renderChild,
					} as never,
				),
			),
		);
	});
	if (fetchError) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		// React Query retains successful data when its refetch fails.
		expect(
			client.getQueryData<typeof inlineWidgetDef>([
				"getWidget",
				"app-1",
				"artikel",
			]),
		).toEqual(inlineWidgetDef);
	}

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

describe("A2UIWidgetInstance cached definitions after a failed refetch", () => {
	for (const status of [404, 410]) {
		test(`removes the cached widget when the server returns ${status}`, async () => {
			const host = await renderInstance(
				() => createElement("div", { "data-external": "cached" }),
				new ApiResponseError({
					status,
					code: "NOT_FOUND",
					message: "Widget no longer exists",
				}),
			);
			expect(host.innerHTML).not.toContain('data-external="cached"');
			expect(host.textContent).toContain("could not be resolved");
		});
	}

	test("keeps the cached widget when a network request fails", async () => {
		const host = await renderInstance(
			() => createElement("div", { "data-external": "cached" }),
			new Error("Network unavailable"),
		);
		expect(host.innerHTML).toContain('data-external="cached"');
	});
});
