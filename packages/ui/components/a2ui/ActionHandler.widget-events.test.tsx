import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { Fragment, StrictMode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import {
	type IRunTimingReport,
	openRunTimingReports,
	recordNativePreamble,
	resetRunTiming,
} from "../../lib/run-timing";
import type { IIntercomEvent } from "../../lib/schema/events/intercom-event";
import type { A2UIServerMessage, Action, SurfaceComponent } from "./types";

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

async function setupTimingHarness(
	executeEvent: (
		appId: string,
		eventId: string,
		run: unknown,
		stream: boolean,
		onStarted?: (id: string) => void,
		onEvents?: (events: IIntercomEvent[]) => void,
	) => Promise<undefined>,
	onA2UIMessage?: (message: A2UIServerMessage) => void,
	strictMode = false,
) {
	const harness = await setupPageHarness();
	const {
		ActionProvider,
		WidgetInstanceProvider,
		AppRouterContext,
		PathnameContext,
		Probe,
		controls,
		setBackend,
	} = harness;
	const holder = globalThis as { __flowLikeRunTimings?: IRunTimingReport[] };
	const previousReports = holder.__flowLikeRunTimings;
	holder.__flowLikeRunTimings = [];
	resetRunTiming();
	const info = spyOn(console, "info").mockImplementation(() => {});
	cleanup.push(() => {
		resetRunTiming();
		holder.__flowLikeRunTimings = previousReports;
		info.mockRestore();
	});
	setBackend({ boardState: {}, eventState: { executeEvent } });
	const Mode = strictMode ? StrictMode : Fragment;
	const render = (
		showControl = true,
		components: Record<string, SurfaceComponent> = {},
	) =>
		act(async () => {
			root?.render(
				<Mode>
					<AppRouterContext.Provider value={{} as never}>
						<PathnameContext.Provider value="/use">
							<ActionProvider
								appId="app-action-timing"
								surfaceId="page-history"
								eventId="page-event"
								governedPage
								isPreviewMode
								elementDemand={{ selectors: [] }}
								components={components}
								onA2UIMessage={onA2UIMessage}
							>
								<WidgetInstanceProvider
									instanceId="person-card"
									widgetId="person"
									actionBindings={{
										open: {
											workflow: { flowId: "history", inputMappings: {} },
											pageAction: {
												actionId: "history",
												manifestRevision: "revision-1",
											},
										},
									}}
								>
									{showControl && <Probe id="card" />}
								</WidgetInstanceProvider>
							</ActionProvider>
						</PathnameContext.Provider>
					</AppRouterContext.Provider>
				</Mode>,
			);
		});
	await render();
	return {
		executeAction: controls.card.executeAction,
		reports: () => holder.__flowLikeRunTimings ?? [],
		render,
	};
}

describe("widget and workflow action timing", () => {
	for (const name of ["widget_event", "workflow_event"] as const) {
		test(`${name} records the first applied update before the run settles`, async () => {
			let onEvents: ((events: IIntercomEvent[]) => void) | undefined;
			let resolveStarted!: () => void;
			const started = new Promise<void>((resolve) => {
				resolveStarted = resolve;
			});
			let resolveRun!: (value: undefined) => void;
			const running = new Promise<undefined>((resolve) => {
				resolveRun = resolve;
			});
			let applied = 0;
			const { executeAction, reports } = await setupTimingHarness(
				async (_app, _event, _run, _stream, onStarted, receive) => {
					recordNativePreamble("run-history", {
						total_ms: 12,
						steps: [{ step: "open_board", ms: 12 }],
					});
					onStarted?.("run-history");
					onEvents = receive;
					resolveStarted();
					return running;
				},
				() => {
					if (applied === 0) {
						const marks = openRunTimingReports()[0].marks.map(
							(mark) => mark.name,
						);
						expect(marks).toContain("first_a2ui_update_received");
						expect(marks).not.toContain("first_a2ui_update_applied");
					}
					applied += 1;
				},
			);
			const action: Action =
				name === "widget_event"
					? { name, context: { actionId: "open" } }
					: {
							name,
							context: {},
							pageAction: {
								actionId: "history",
								manifestRevision: "revision-1",
							},
						};
			let pending!: Promise<void>;
			try {
				await act(async () => {
					pending = executeAction(action, "history-button", {
						privateValue: "hidden",
					});
					await started;
				});
				const event = (payload: unknown): IIntercomEvent => ({
					event_id: "update",
					event_type: "a2ui",
					payload,
					timestamp: { secs_since_epoch: 0, nanos_since_epoch: 0 },
				});
				onEvents?.([event({ type: "requestElements", element_ids: [] })]);
				expect(
					openRunTimingReports()[0].marks.map((mark) => mark.name),
				).toEqual(["dispatch", "run_initiated"]);
				const update = event({
					type: "surfaceUpdate",
					surfaceId: "page-history",
					components: [],
				});
				onEvents?.([update, update]);
				onEvents?.([update]);
				expect(applied).toBe(3);
				expect(reports()).toEqual([]);
				expect(
					openRunTimingReports()[0].marks.map((mark) => mark.name),
				).toEqual([
					"dispatch",
					"run_initiated",
					"first_a2ui_update_received",
					"first_a2ui_update_applied",
				]);
			} finally {
				resolveRun(undefined);
				await act(async () => pending);
			}
			expect(openRunTimingReports()).toEqual([]);
			expect(reports()).toHaveLength(1);
			const [report] = reports();
			expect(report.label).toBe(
				name === "widget_event" ? "widget action" : "workflow action",
			);
			expect(report.marks.at(-1)?.name).toBe("settled");
			expect(report.steps.map((step) => step.name)).toEqual([
				"action.collect_elements",
				"action.frontend_context",
			]);
			expect(report.native).toEqual({
				totalMs: 12,
				steps: [{ step: "open_board", ms: 12 }],
			});
			expect(JSON.stringify(report)).not.toContain("hidden");
		});
	}

	test("rejected and unbound actions close their traces", async () => {
		const failure = new Error("execution rejected");
		const { executeAction, reports } = await setupTimingHarness(async () => {
			throw failure;
		});
		await act(async () => {
			await expect(
				executeAction(
					{ name: "widget_event", context: { actionId: "open" } },
					"history-button",
					{},
					{ requireSuccess: true },
				),
			).rejects.toBe(failure);
			await executeAction({
				name: "widget_event",
				context: { actionId: "unbound" },
			});
		});
		expect(openRunTimingReports()).toEqual([]);
		expect(reports()).toHaveLength(2);
		expect(reports()[0].marks.map((mark) => mark.name)).toEqual([
			"dispatch",
			"settled",
		]);
		expect(reports()[1].marks.map((mark) => mark.name)).toEqual(["settled"]);
	});
});

describe("page widget event dispatch", () => {
	test.each([false, true])(
		"a widget run reads only while its page is mounted (StrictMode: %s)",
		async (strictMode) => {
			let onEvents: ((events: IIntercomEvent[]) => void) | undefined;
			let resolveStarted!: () => void;
			const started = new Promise<void>((resolve) => {
				resolveStarted = resolve;
			});
			let resolveRun!: (value: undefined) => void;
			const running = new Promise<undefined>((resolve) => {
				resolveRun = resolve;
			});
			const { executeAction, render } = await setupTimingHarness(
				async (_app, _event, _run, _stream, _onStarted, receive) => {
					onEvents = receive;
					resolveStarted();
					return running;
				},
				undefined,
				strictMode,
			);
			const channels = await import("../../lib/channel");
			const reply = spyOn(channels, "replyToChannel").mockResolvedValue();
			cleanup.push(() => reply.mockRestore());
			const headline = (value: string): SurfaceComponent => ({
				id: "headline",
				component: {
					id: "headline",
					type: "text",
					content: { literalString: value },
				},
			});
			const initial = headline("Current page");
			await render(true, { headline: initial });
			const request = (id: string): IIntercomEvent[] => [
				{
					event_id: id,
					event_type: "a2ui",
					payload: {
						type: "requestElements",
						request_id: id,
						selectors: ["type:text"],
						timeout_ms: 15_000,
						channel: {
							channel_id: "widget-reload-channel",
							request_id: id,
							expires_at: Math.floor(Date.now() / 1000) + 15,
							transport: { type: "in_process" },
						},
					},
					timestamp: { secs_since_epoch: 0, nanos_since_epoch: 0 },
				},
			];
			let pending!: Promise<void>;
			try {
				await act(async () => {
					pending = executeAction({
						name: "widget_event",
						context: { actionId: "open" },
					});
					await started;
				});
				await act(async () => onEvents?.(request("before-unmount")));
				expect(reply.mock.calls[0]?.[1]).toEqual({
					ok: true,
					elements: { "page-history/headline": initial },
				});
				const updated = headline("Updated after the control left");
				await render(false, { headline: updated });
				await act(async () => onEvents?.(request("after-control-unmount")));
				expect(reply.mock.calls[1]?.[1]).toEqual({
					ok: true,
					elements: { "page-history/headline": updated },
				});

				await act(() => root?.unmount());
				root = undefined;
				await act(async () => onEvents?.(request("after-unmount")));
				expect(reply.mock.calls[2]?.[1]).toMatchObject({
					ok: false,
					error: expect.any(String),
				});
				expect(reply).toHaveBeenCalledTimes(3);
			} finally {
				resolveRun(undefined);
				await act(async () => pending);
			}
		},
	);

	test("a captured widget activation keeps its original instance when named handlers delegate", async () => {
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
		setBackend({
			boardState: {},
			eventState: {
				executeEvent: async (
					_appId: string,
					_eventId: string,
					run: { id: string; payload: Record<string, unknown> },
				) => {
					runs.push(run);
				},
			},
		});
		const render = (instanceId: string) =>
			act(async () => {
				root?.render(
					<AppRouterContext.Provider value={{} as never}>
						<PathnameContext.Provider value="/use">
							<ActionProvider
								appId="app-widget-activation"
								surfaceId="page-cards"
								eventId="page-event"
								governedPage
								isPreviewMode
								components={{}}
							>
								<WidgetInstanceProvider
									instanceId={instanceId}
									widgetId="card"
									componentId={instanceId}
									actionBindings={{}}
									eventHandlers={{
										open: [
											{
												name: "workflow_event",
												context: {},
												pageAction: {
													actionId: `open-${instanceId}`,
													manifestRevision: "revision-1",
												},
											},
										],
									}}
								>
									<Probe id="widget" />
								</WidgetInstanceProvider>
							</ActionProvider>
						</PathnameContext.Provider>
					</AppRouterContext.Provider>,
				);
			});

		await render("card-a");
		const pressed = controls.widget.executeAction;
		await render("card-b");
		expect(controls.widget.executeAction).not.toBe(pressed);
		const action: Action = {
			name: "widget_event",
			context: { actionId: "open" },
		};
		await act(async () => {
			await pressed(action, "open-button");
			await controls.widget.executeAction(action, "open-button");
		});

		expect(runs).toHaveLength(2);
		for (const [index, instanceId] of ["card-a", "card-b"].entries()) {
			expect(runs[index].id).toBe(`open-${instanceId}`);
			expect(runs[index].payload._widget_instance_id).toBe(instanceId);
			expect(runs[index].payload._triggering_component_id).toBe(instanceId);
			expect(runs[index].payload._action_id).toBe("open");
		}
	});

	test("an unbound widget event warns only when its widget does not declare it", async () => {
		const {
			ActionProvider,
			WidgetInstanceProvider,
			AppRouterContext,
			PathnameContext,
			Probe,
			controls,
			setBackend,
		} = await setupPageHarness();
		const { toast } = await import("sonner");
		const toastWarning = spyOn(toast, "warning").mockImplementation(() => "");
		const warn = spyOn(console, "warn").mockImplementation(() => {});
		const debug = spyOn(console, "debug").mockImplementation(() => {});
		cleanup.push(() => {
			toastWarning.mockRestore();
			warn.mockRestore();
			debug.mockRestore();
		});
		setBackend({
			boardState: {},
			eventState: {
				executeEvent: async () => {
					throw new Error("An unbound widget event must not start a run.");
				},
			},
		});
		await act(async () => {
			root?.render(
				<AppRouterContext.Provider value={{} as never}>
					<PathnameContext.Provider value="/use">
						<ActionProvider
							appId="app-undeclared-widget-event"
							surfaceId="page-cards"
							eventId="page-event"
							governedPage
							isPreviewMode
							components={{}}
						>
							<WidgetInstanceProvider
								instanceId="card"
								widgetId="card"
								actionBindings={{}}
								declaredActionIds={["open"]}
							>
								<Probe id="widget" />
							</WidgetInstanceProvider>
						</ActionProvider>
					</PathnameContext.Provider>
				</AppRouterContext.Provider>,
			);
		});
		const fire = (actionId: string, requireSuccess = false) =>
			controls.widget.executeAction(
				{ name: "widget_event", context: { actionId } },
				"card-button",
				{},
				{ requireSuccess },
			);

		await act(async () => {
			await fire("open");
		});
		expect(toastWarning).not.toHaveBeenCalled();

		await act(async () => {
			await fire("ghost");
		});
		expect(toastWarning).toHaveBeenCalledTimes(1);
		expect(String(toastWarning.mock.calls[0][0])).toContain("ghost");
		expect(
			warn.mock.calls.some(([message]) => String(message).includes('"ghost"')),
		).toBe(true);

		await act(async () => {
			await expect(fire("ghost", true)).rejects.toThrow(
				'The widget event "ghost" is not declared by its widget.',
			);
		});
	});

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
