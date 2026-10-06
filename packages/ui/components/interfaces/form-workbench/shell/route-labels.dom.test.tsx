import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import type { IEvent } from "../../../../lib/schema/flow/event";
import type { IRouteMapping } from "../../../../state/backend-state/route-state";
import { installWorkbenchDom, settle } from "../testing/dom";

const dom = installWorkbenchDom();
const { useBackendStore } = await import("../../../../state/backend-state");
const { useRouteLabels } = await import("./route-labels");

afterEach(async () => {
	await dom.cleanup();
	useBackendStore.setState({ backend: null });
});
afterAll(dom.restore);

interface Calls {
	routes: string[];
	events: string[];
}

function backendWith(
	mappings: readonly IRouteMapping[],
	events: readonly Pick<IEvent, "id" | "name">[],
	calls: Calls,
) {
	return {
		routeState: {
			getRoutes: async (appId: string) => {
				calls.routes.push(appId);
				return [...mappings];
			},
		},
		eventState: {
			getEvents: async (appId: string) => {
				calls.events.push(appId);
				return [...events];
			},
		},
	};
}

function Labels({
	appId,
	routes,
}: Readonly<{ appId: string; routes: readonly string[] }>) {
	const labels = useRouteLabels(appId, routes);
	return <output data-labels={JSON.stringify(labels)} />;
}

function withClient(node: ReactNode) {
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	return <QueryClientProvider client={client}>{node}</QueryClientProvider>;
}

const labelsIn = (container: Element) =>
	JSON.parse(
		container.querySelector("output")?.getAttribute("data-labels") ?? "{}",
	);

describe("useRouteLabels", () => {
	test("reads nothing for a form without routes", async () => {
		const calls: Calls = { routes: [], events: [] };
		useBackendStore.getState().setBackend(backendWith([], [], calls) as never);
		const view = await dom.render(
			withClient(<Labels appId="app-1" routes={[]} />),
		);
		await settle();
		expect(calls).toEqual({ routes: [], events: [] });
		expect(labelsIn(view.container)).toEqual({});
	});

	test("labels come from the path until a backend is ready, and nothing is called", async () => {
		const view = await dom.render(
			withClient(<Labels appId="app-1" routes={["/", "/review-queue"]} />),
		);
		await settle();
		expect(labelsIn(view.container)).toEqual({
			"/": "Home",
			"/review-queue": "Review queue",
		});
	});

	test("takes the name of the event each route is mapped to", async () => {
		const calls: Calls = { routes: [], events: [] };
		useBackendStore.getState().setBackend(
			backendWith(
				[
					{ path: "/review", eventId: "e1" },
					{ path: "/chat", eventId: "e2" },
				],
				[
					{ id: "e1", name: "Review queue" },
					{ id: "e2", name: "Support chat" },
				],
				calls,
			) as never,
		);
		const view = await dom.render(
			withClient(<Labels appId="app-1" routes={["/", "/review", "/chat"]} />),
		);
		await settle();
		await settle();
		expect(calls.routes).toEqual(["app-1"]);
		expect(calls.events).toEqual(["app-1"]);
		expect(labelsIn(view.container)).toEqual({
			"/": "Home",
			"/review": "Review queue",
			"/chat": "Support chat",
		});
	});
});
