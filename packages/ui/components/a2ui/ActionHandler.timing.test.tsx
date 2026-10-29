import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import {
	type IRunTimingReport,
	openRunTimingReports,
	resetRunTiming,
} from "../../lib/run-timing";
import type { IIntercomEvent } from "../../lib/schema/events/intercom-event";
import type { IEventState } from "../../state/backend-state/event-state";
import type { Action } from "./types";

let root: Root | undefined;
const cleanup: (() => void)[] = [];

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	for (const restore of cleanup.reverse()) restore();
	cleanup.length = 0;
	resetRunTiming();
});

const pageAction = {
	actionId: "private-action",
	manifestRevision: "revision-1",
};
const workflow: Action = { name: "workflow_event", context: {}, pageAction };
const widget: Action = { name: "widget_event", context: { actionId: "open" } };

function a2ui(payload: Record<string, unknown>): IIntercomEvent[] {
	return [{ event_type: "a2ui", payload } as IIntercomEvent];
}

async function setup(executeEvent: IEventState["executeEvent"]) {
	const window = new Window({ url: "https://local/use" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
		__flowLikeRunTimings: [] as IRunTimingReport[],
		__flowLikeRunTimingsOpen: undefined as (() => string) | undefined,
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
		{ toast },
	] = await Promise.all([
		import("./ActionHandler"),
		import("./layout/A2UIWidgetInstance"),
		import("../../state/backend-state"),
		import("../../lib/idb-storage"),
		import("../../db/ui-state-db"),
		import("next/dist/shared/lib/app-router-context.shared-runtime"),
		import("next/dist/shared/lib/hooks-client-context.shared-runtime"),
		import("sonner"),
	]);
	let now = 1000;
	const spies = [
		spyOn(appGlobalState, "getAll").mockResolvedValue({}),
		spyOn(pageLocalState, "getAll").mockResolvedValue({}),
		spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
		spyOn(uiState, "pruneElementValues").mockResolvedValue(),
		spyOn(console, "log").mockImplementation(() => {}),
		spyOn(console, "warn").mockImplementation(() => {}),
		spyOn(console, "error").mockImplementation(() => {}),
		spyOn(console, "info").mockImplementation(() => {}),
		spyOn(toast, "error").mockImplementation(() => ""),
		spyOn(performance, "now").mockImplementation(() => now),
	];
	cleanup.push(() => {
		for (const spy of spies) spy.mockRestore();
	});
	const previousBackend = useBackendStore.getState().backend;
	cleanup.push(() => useBackendStore.setState({ backend: previousBackend }));
	useBackendStore.getState().setBackend({
		boardState: {},
		eventState: { executeEvent },
	} as never);

	const controls: Record<string, ReturnType<typeof useExecuteAction>> = {};
	function Probe({ id }: { id: string }) {
		controls[id] = useExecuteAction();
		return null;
	}
	const delivered: unknown[] = [];
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);
	await act(async () => {
		root?.render(
			<AppRouterContext.Provider
				value={{ push: () => delivered.push("navigate") } as never}
			>
				<PathnameContext.Provider value="/use">
					<ActionProvider
						appId="private-app"
						surfaceId="private-page"
						eventId="private-event"
						governedPage
						isPreviewMode
						components={{}}
						onA2UIMessage={(message) => delivered.push(message)}
						openDialog={() => {
							delivered.push("open");
							now += 50;
						}}
						closeDialog={() => delivered.push("close")}
					>
						<WidgetInstanceProvider
							instanceId="private-widget"
							widgetId="card"
							componentId="private-widget"
							actionBindings={{
								open: {
									workflow: { flowId: "private-node", inputMappings: {} },
									pageAction,
								},
							}}
						>
							<Probe id="widget" />
						</WidgetInstanceProvider>
						<Probe id="page" />
					</ActionProvider>
				</PathnameContext.Provider>
			</AppRouterContext.Provider>,
		);
	});
	resetRunTiming();
	return {
		controls,
		delivered,
		clock: (value: number) => {
			now = value;
		},
		reports: () => globals.__flowLikeRunTimings,
	};
}

const markTime = (report: IRunTimingReport, name: string) =>
	report.marks.find((mark) => mark.name === name)?.atMs;

describe("action run timings", () => {
	test("records preparation, first dialog receipt and application without logging context", async () => {
		const harness = await setup(
			async (_app, _event, _payload, _stream, onStarted, onEvents) => {
				harness.clock(1100);
				onStarted?.("private-run");
				onEvents?.(a2ui({ type: "requestElements", request_id: "request" }));
				onEvents?.(
					a2ui({
						type: "widgetQuery",
						requestId: "query",
						instanceId: "private-widget",
						query: "read",
					}),
				);
				harness.clock(1200);
				onEvents?.(a2ui({ type: "openDialog", route: "/private-panel" }));
				harness.clock(1300);
				onEvents?.(a2ui({ type: "showScreen" }));
				harness.clock(1400);
				return undefined;
			},
		);
		await act(async () => {
			await harness.controls.page.executeAction(workflow, "private-button", {
				private_value: "secret payload",
			});
		});
		const [report] = harness.reports();
		expect(harness.reports()).toHaveLength(1);
		expect(report.label).toBe("workflow action");
		expect(report.steps.map((step) => step.name)).toEqual([
			"action.collect_elements",
			"action.frontend_context",
		]);
		expect(markTime(report, "run_initiated")).toBe(100);
		expect(markTime(report, "first_a2ui_update_received")).toBe(200);
		expect(markTime(report, "first_a2ui_update_applied")).toBe(250);
		expect(markTime(report, "settled")).toBe(400);
		expect(harness.delivered).toEqual(["open", { type: "showScreen" }]);
		expect(JSON.stringify(report)).not.toContain("private");
		expect(JSON.stringify(report)).not.toContain("secret payload");
		expect(openRunTimingReports()).toEqual([]);
	});

	test("records navigation handled directly without a forwarded surface message", async () => {
		const harness = await setup(
			async (_app, _event, _payload, _stream, _onStarted, onEvents) => {
				harness.clock(1250);
				onEvents?.(
					a2ui({ type: "navigateTo", route: "/destination", replace: false }),
				);
				return undefined;
			},
		);
		await act(async () => {
			await harness.controls.page.executeAction(workflow);
		});
		const [report] = harness.reports();
		expect(markTime(report, "first_a2ui_update_received")).toBe(250);
		expect(markTime(report, "first_a2ui_update_applied")).toBe(250);
		expect(harness.delivered).toEqual(["navigate"]);
	});

	test("concurrent widget and workflow runs retain their own first output and lifetime", async () => {
		const pending: {
			onEvents: NonNullable<Parameters<IEventState["executeEvent"]>[5]>;
			finish: () => void;
		}[] = [];
		const harness = await setup(
			(_app, _event, _payload, _stream, _onStarted, onEvents) =>
				new Promise((resolve) => {
					if (!onEvents) throw new Error("Missing execution callback");
					pending.push({ onEvents, finish: () => resolve(undefined) });
				}),
		);
		let workflowRun!: Promise<void>;
		let widgetRun!: Promise<void>;
		await act(async () => {
			workflowRun = harness.controls.page.executeAction(workflow);
			widgetRun = harness.controls.widget.executeAction(widget);
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		expect(pending).toHaveLength(2);
		expect(openRunTimingReports()).toHaveLength(2);
		await act(async () => {
			harness.clock(1200);
			pending[1].onEvents(a2ui({ type: "showScreen" }));
			harness.clock(1300);
			pending[1].finish();
			await widgetRun;
		});
		expect(openRunTimingReports()).toHaveLength(1);
		await act(async () => {
			harness.clock(1400);
			pending[0].onEvents(a2ui({ type: "closeDialog" }));
			harness.clock(1500);
			pending[0].finish();
			await workflowRun;
		});
		const [widgetReport, workflowReport] = harness.reports();
		expect(widgetReport.label).toBe("widget action");
		expect(markTime(widgetReport, "first_a2ui_update_received")).toBe(200);
		expect(widgetReport.totalMs).toBe(300);
		expect(workflowReport.label).toBe("workflow action");
		expect(markTime(workflowReport, "first_a2ui_update_received")).toBe(400);
		expect(workflowReport.totalMs).toBe(500);
		expect(openRunTimingReports()).toEqual([]);
	});

	test("finishes refused and rejected actions, without tracing non-workflow actions", async () => {
		const harness = await setup(async () => {
			throw new Error("failure");
		});
		await act(async () => {
			await harness.controls.widget.executeAction({
				name: "widget_event",
				context: { actionId: "unbound" },
			});
			await harness.controls.page.executeAction(workflow);
			await expect(
				harness.controls.page.executeAction(
					workflow,
					undefined,
					{},
					{
						requireSuccess: true,
					},
				),
			).rejects.toThrow("failure");
			await harness.controls.page.executeAction({
				name: "workflow_event",
				context: {},
			});
			await harness.controls.page.executeAction({
				name: "external_link",
				context: {},
			});
		});
		expect(harness.reports()).toHaveLength(4);
		for (const report of harness.reports()) {
			expect(markTime(report, "settled")).toBeDefined();
			expect(markTime(report, "first_a2ui_update_received")).toBeUndefined();
		}
		expect(openRunTimingReports()).toEqual([]);
	});
});
