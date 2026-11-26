import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";
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

type SurfaceKind = "page" | "editor";

interface RenderState {
	/** The Page is showing a restored bootstrap and its execution authority is not confirmed yet. */
	pending?: boolean;
	surface?: SurfaceKind;
	widgetMounted?: boolean;
}

async function setupPendingPage() {
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
		spyOn(console, "log").mockImplementation(() => {}),
		spyOn(console, "info").mockImplementation(() => {}),
	];
	cleanup.push(() => {
		for (const spy of spies) spy.mockRestore();
	});

	const previousBackend = useBackendStore.getState().backend;
	cleanup.push(() => useBackendStore.setState({ backend: previousBackend }));
	const executeEvent = mock(
		async (
			_appId: string,
			_eventId: string,
			_payload: { id: string; payload: Record<string, unknown> },
		): Promise<undefined> => undefined,
	);
	useBackendStore
		.getState()
		.setBackend({ boardState: {}, eventState: { executeEvent } } as never);

	const controls: Record<string, ReturnType<typeof useExecuteAction>> = {};
	function Widget() {
		controls.widget = useExecuteAction();
		return null;
	}

	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);

	const render = ({
		pending = false,
		surface = "page",
		widgetMounted = true,
	}: RenderState = {}) =>
		act(async () => {
			root?.render(
				<AppRouterContext.Provider value={{} as never}>
					<PathnameContext.Provider value="/use">
						<ActionProvider
							appId="app-pending"
							surfaceId="dashboard"
							eventId="page-event"
							governedPage
							isPreviewMode={surface === "page" && !pending}
							executionPending={surface === "page" && pending}
							elementDemand={{ selectors: [] }}
							components={{}}
						>
							<WidgetInstanceProvider
								instanceId="sales-chart"
								widgetId="chart"
								actionBindings={{
									load: {
										workflow: { flowId: "load-sales", inputMappings: {} },
										pageAction: {
											actionId: "pa1_load_sales",
											manifestRevision: "revision-1",
										},
									},
								}}
							>
								{widgetMounted && <Widget />}
							</WidgetInstanceProvider>
						</ActionProvider>
					</PathnameContext.Provider>
				</AppRouterContext.Provider>,
			);
		});

	const emit = (action: Action = loadEvent) => {
		let settled = false;
		const done = controls.widget
			.executeAction(action, "sales-chart", {}, { origin: "micro_widget" })
			.then(() => {
				settled = true;
			});
		return { done, settled: () => settled };
	};

	const save = () =>
		controls.widget.executeAction(
			loadEvent,
			"sales-chart",
			{},
			{ requireSuccess: true },
		);

	return { executeEvent, render, emit, save };
}

const loadEvent: Action = {
	name: "widget_event",
	context: { actionId: "load" },
};

describe("actions sent while a Page's execution authority is pending", () => {
	test("a widget event waits for the authority and then runs once", async () => {
		const { executeEvent, render, emit } = await setupPendingPage();
		await render({ pending: true });

		let sent!: ReturnType<typeof emit>;
		await act(async () => {
			sent = emit();
			await Promise.resolve();
		});
		expect(executeEvent).not.toHaveBeenCalled();
		expect(sent.settled()).toBe(false);

		await render({ pending: false });
		await act(async () => sent.done);

		expect(executeEvent).toHaveBeenCalledTimes(1);
		const [appId, eventId, run] = executeEvent.mock.calls[0];
		expect(appId).toBe("app-pending");
		expect(eventId).toBe("page-event");
		expect(run.id).toBe("pa1_load_sales");
		expect(run.payload._action_id).toBe("load");
		expect(run.payload._widget_instance_id).toBe("sales-chart");
	});

	test("events keep their order when the authority is confirmed", async () => {
		const { executeEvent, render, emit } = await setupPendingPage();
		await render({ pending: true });

		let first!: ReturnType<typeof emit>;
		let second!: ReturnType<typeof emit>;
		await act(async () => {
			first = emit();
			second = emit({
				name: "widget_event",
				context: { actionId: "load", range: "week" },
			});
		});
		await render({ pending: false });
		await act(async () => Promise.all([first.done, second.done]));

		expect(executeEvent).toHaveBeenCalledTimes(2);
		expect(
			executeEvent.mock.calls.map(
				([, , run]) =>
					(run.payload._action_context as Record<string, unknown>).range,
			),
		).toEqual([undefined, "week"]);
	});

	test("an event whose widget unmounts before the authority is confirmed never runs", async () => {
		const { executeEvent, render, emit } = await setupPendingPage();
		await render({ pending: true });

		let sent!: ReturnType<typeof emit>;
		await act(async () => {
			sent = emit();
		});
		await render({ pending: true, widgetMounted: false });
		await act(async () => sent.done);
		await render({ pending: false, widgetMounted: false });

		expect(sent.settled()).toBe(true);
		expect(executeEvent).not.toHaveBeenCalled();
	});

	test("a required-success action fails when its widget unmounts before the authority is confirmed", async () => {
		const { executeEvent, render, save } = await setupPendingPage();
		await render({ pending: true });

		let outcome!: Promise<unknown>;
		await act(async () => {
			outcome = save().then(
				() => "ran",
				(error: Error) => error.message,
			);
		});
		await render({ pending: true, widgetMounted: false });
		expect(await outcome).toBe(
			"This workflow action is unavailable outside the running page.",
		);
		expect(executeEvent).not.toHaveBeenCalled();
	});

	test("a required-success action sent while pending resolves with its run", async () => {
		const { executeEvent, render, save } = await setupPendingPage();
		executeEvent.mockImplementation(async (...args: unknown[]) => {
			const onStarted = args[4] as ((runId: string) => void) | undefined;
			const onEvents = args[5] as
				| ((events: Record<string, unknown>[]) => void)
				| undefined;
			onStarted?.("run-save");
			onEvents?.([
				{
					event_id: "done",
					event_type: "completed",
					payload: { status: "success" },
					timestamp: { secs_since_epoch: 0, nanos_since_epoch: 0 },
				},
			]);
			return undefined;
		});
		await render({ pending: true });

		let outcome!: Promise<void>;
		await act(async () => {
			outcome = save();
		});
		await render({ pending: false });
		await act(async () => outcome);
		expect(executeEvent).toHaveBeenCalledTimes(1);
	});

	test("an editor surface still never runs actions", async () => {
		const { executeEvent, render, emit } = await setupPendingPage();
		await render({ surface: "editor" });

		let sent!: ReturnType<typeof emit>;
		await act(async () => {
			sent = emit();
			await sent.done;
		});
		await render({ surface: "editor" });

		expect(sent.settled()).toBe(true);
		expect(executeEvent).not.toHaveBeenCalled();
	});
});
