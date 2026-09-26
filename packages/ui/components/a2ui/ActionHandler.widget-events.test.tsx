import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { Action } from "./types";

let root: Root | undefined;
const cleanup: (() => void)[] = [];

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	for (const restore of cleanup.reverse()) restore();
	cleanup.length = 0;
});

/** Install happy-dom globals and the storage spies, then load the modules under test. */
async function setupPageHarness() {
	const window = new Window({ url: "https://local/use" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	cleanup.push(() => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	});

	const [
		{ ActionProvider, useExecuteAction },
		{ WidgetInstanceProvider },
		{ useBackendStore },
		{ appGlobalState, pageLocalState },
		uiState,
		{ AppRouterContext },
		{ PathnameContext },
	] = await Promise.all([
		import("./ActionHandler"),
		import("./layout/A2UIWidgetInstance"),
		import("../../state/backend-state"),
		import("../../lib/idb-storage"),
		import("../../db/ui-state-db"),
		import("next/dist/shared/lib/app-router-context.shared-runtime"),
		import("next/dist/shared/lib/hooks-client-context.shared-runtime"),
	]);
	const spies = [
		spyOn(appGlobalState, "getAll").mockResolvedValue({}),
		spyOn(pageLocalState, "getAll").mockResolvedValue({}),
		spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
		spyOn(uiState, "pruneElementValues").mockResolvedValue(),
	];
	cleanup.push(() => {
		for (const spy of spies) spy.mockRestore();
	});

	const previousBackend = useBackendStore.getState().backend;
	cleanup.push(() => useBackendStore.setState({ backend: previousBackend }));

	const controls: Record<string, ReturnType<typeof useExecuteAction>> = {};
	function Probe({ id }: { id: string }) {
		controls[id] = useExecuteAction();
		return null;
	}

	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);

	return {
		ActionProvider,
		WidgetInstanceProvider,
		AppRouterContext,
		PathnameContext,
		Probe,
		controls,
		setBackend: (backend: unknown) =>
			useBackendStore.getState().setBackend(backend as never),
	};
}

describe("page widget event dispatch", () => {
	test("named handlers and saved bindings carry the event and originating instance to the authorized Page run", async () => {
		const {
			ActionProvider,
			WidgetInstanceProvider,
			AppRouterContext,
			PathnameContext,
			Probe,
			controls,
			setBackend,
		} = await setupPageHarness();

		const runs: { id: string; payload: Record<string, unknown> }[] = [];
		const triggers: unknown[] = [];
		let rawBoardRuns = 0;
		setBackend({
			boardState: {
				executeBoard: async () => {
					rawBoardRuns++;
				},
			},
			eventState: {
				executeEvent: async (
					appId: string,
					eventId: string,
					run: { id: string; payload: Record<string, unknown> },
					_stream: boolean,
					onStarted?: (id: string) => void,
					_onEvents?: unknown,
					_version?: unknown,
					trigger?: unknown,
				) => {
					expect(appId).toBe("app-widget-events");
					expect(eventId).toBe("page-event");
					runs.push(run);
					triggers.push(trigger);
					onStarted?.("run-widget-events");
				},
			},
		});

		const pageAction = {
			actionId: "bound-click",
			manifestRevision: "revision-1",
		};
		await act(async () => {
			root?.render(
				<AppRouterContext.Provider value={{} as never}>
					<PathnameContext.Provider value="/use">
						<ActionProvider
							appId="app-widget-events"
							surfaceId="page-map"
							eventId="page-event"
							governedPage
							isPreviewMode
							components={{}}
						>
							{["map-a", "map-b"].map((id) => (
								<WidgetInstanceProvider
									key={id}
									instanceId={id}
									widgetId="gods-eye-view"
									componentId={id}
									actionBindings={{
										entityClicked: {
											workflow: { flowId: "handler-node", inputMappings: {} },
											pageAction,
										},
									}}
								>
									<Probe id={id} />
								</WidgetInstanceProvider>
							))}
							<Probe id="page" />
						</ActionProvider>
					</PathnameContext.Provider>
				</AppRouterContext.Provider>,
			);
		});

		const entity = {
			type: "Feature",
			id: "camera-1",
			geometry: { type: "Point", coordinates: [13.4, 52.5] },
			properties: { kind: "camera" },
		};
		const event = { entity, source: "globe" };
		const context = { ...event, actionId: "entityClicked", payload: event };
		const named: Action = {
			name: "workflow_event",
			context: { nodeId: "handler-node" },
			pageAction,
		};
		await act(async () => {
			await controls["map-a"].executeAction(named, "map-a", context);
			await controls["map-b"].executeAction(named, "map-b", context);
			await controls["map-a"].executeAction(
				{ name: "widget_event", context },
				"map-a",
			);
			await controls.page.executeAction(named, "page-button", {
				actionId: "ordinary-page-field",
			});
		});

		expect(runs).toHaveLength(4);
		expect(rawBoardRuns).toBe(0);
		for (const [index, instanceId] of ["map-a", "map-b", "map-a"].entries()) {
			expect(runs[index].id).toBe("bound-click");
			expect(runs[index].payload._action_id).toBe("entityClicked");
			expect(runs[index].payload._widget_instance_id).toBe(instanceId);
			expect(runs[index].payload._triggering_component_id).toBe(instanceId);
			expect(runs[index].payload._action_context).toMatchObject({
				entity,
				payload: event,
			});
			expect(triggers[index]).toEqual({
				kind: "action",
				actionId: "bound-click",
				manifestRevision: "revision-1",
			});
		}
		expect(runs[3].payload._widget_instance_id).toBe("");
		expect(runs[3].payload._action_id).toBeUndefined();
	});

	test("an ungoverned widget payload cannot retarget a workflow_event route", async () => {
		const {
			ActionProvider,
			WidgetInstanceProvider,
			AppRouterContext,
			PathnameContext,
			Probe,
			controls,
			setBackend,
		} = await setupPageHarness();

		const boardRuns: {
			appId: string;
			boardId: string;
			payload: { id: string; payload: Record<string, unknown> };
		}[] = [];
		let eventRuns = 0;
		setBackend({
			boardState: {
				executeBoard: async (
					appId: string,
					boardId: string,
					payload: { id: string; payload: Record<string, unknown> },
				) => {
					boardRuns.push({ appId, boardId, payload });
					return undefined;
				},
				getBoard: async () => {
					throw new Error("not needed");
				},
			},
			eventState: {
				executeEvent: async () => {
					eventRuns++;
				},
			},
		});

		await act(async () => {
			root?.render(
				<AppRouterContext.Provider value={{} as never}>
					<PathnameContext.Provider value="/use">
						<ActionProvider
							appId="app-widget-events"
							boardId="board-authored"
							surfaceId="page-map"
							isPreviewMode
							components={{}}
						>
							<WidgetInstanceProvider
								instanceId="map-a"
								widgetId="gods-eye-view"
								componentId="map-a"
								actionBindings={{}}
							>
								<Probe id="map-a" />
							</WidgetInstanceProvider>
						</ActionProvider>
					</PathnameContext.Provider>
				</AppRouterContext.Provider>,
			);
		});

		await act(async () => {
			await controls["map-a"].executeAction(
				{ name: "workflow_event", context: { nodeId: "authored-node" } },
				"map-a",
				{
					version: 1,
					requests: [],
					nodeId: "attacker-node",
					boardId: "attacker-board",
					appId: "attacker-app",
					actionId: "requestMapSource",
					payload: { version: 1, requests: [] },
				},
			);
		});

		expect(boardRuns).toHaveLength(1);
		expect(boardRuns[0].appId).toBe("app-widget-events");
		expect(boardRuns[0].boardId).toBe("board-authored");
		expect(boardRuns[0].payload.id).toBe("authored-node");
		expect(boardRuns[0].payload.payload._action_id).toBe("requestMapSource");
		expect(boardRuns[0].payload.payload._widget_instance_id).toBe("map-a");
		expect(eventRuns).toBe(0);
	});
});
