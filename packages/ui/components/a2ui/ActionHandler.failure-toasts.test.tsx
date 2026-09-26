import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { ApiResponseError } from "../../lib/api-error";
import type { LivePageRunRecord } from "./live-page-registry";
import type { Action } from "./types";

let root: Root | undefined;
const cleanup: (() => void)[] = [];

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	for (const restore of cleanup.reverse()) restore();
	cleanup.length = 0;
});

const WORKFLOW_FALLBACK = "The workflow could not be started.";
const WIDGET_FALLBACK = "The widget workflow could not be started.";

const pageAction = { actionId: "bound-click", manifestRevision: "revision-1" };

/** Mount a governed Page whose Event run rejects with `rejection`. */
async function setupFailingPage(rejection: unknown) {
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
		{ toast },
		{ subscribeLivePageRuns },
	] = await Promise.all([
		import("./ActionHandler"),
		import("./layout/A2UIWidgetInstance"),
		import("../../state/backend-state"),
		import("../../lib/idb-storage"),
		import("../../db/ui-state-db"),
		import("next/dist/shared/lib/app-router-context.shared-runtime"),
		import("next/dist/shared/lib/hooks-client-context.shared-runtime"),
		import("sonner"),
		import("./live-page-registry"),
	]);
	const toastError = spyOn(toast, "error").mockImplementation(() => "");
	const toastInfo = spyOn(toast, "info").mockImplementation(() => "");
	const consoleError = spyOn(console, "error").mockImplementation(() => {});
	const runs: LivePageRunRecord[] = [];
	cleanup.push(
		subscribeLivePageRuns("page-failure", (record) => runs.push(record)),
	);
	const spies = [
		spyOn(appGlobalState, "getAll").mockResolvedValue({}),
		spyOn(pageLocalState, "getAll").mockResolvedValue({}),
		spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
		spyOn(uiState, "pruneElementValues").mockResolvedValue(),
		spyOn(console, "log").mockImplementation(() => {}),
		spyOn(console, "warn").mockImplementation(() => {}),
		toastError,
		toastInfo,
		consoleError,
	];
	cleanup.push(() => {
		for (const spy of spies) spy.mockRestore();
	});

	const previousBackend = useBackendStore.getState().backend;
	cleanup.push(() => useBackendStore.setState({ backend: previousBackend }));
	useBackendStore.getState().setBackend({
		boardState: {},
		eventState: {
			// The desktop backend rejects with whatever Tauri `invoke` rejected with.
			executeEvent: () => Promise.reject(rejection),
		},
	} as never);

	const controls: Record<string, ReturnType<typeof useExecuteAction>> = {};
	function Probe({ id }: { id: string }) {
		controls[id] = useExecuteAction();
		return null;
	}

	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);
	await act(async () => {
		root?.render(
			<AppRouterContext.Provider value={{} as never}>
				<PathnameContext.Provider value="/use">
					<ActionProvider
						appId="app-failure-toasts"
						surfaceId="page-failure"
						eventId="page-event"
						governedPage
						isPreviewMode
						components={{}}
					>
						<WidgetInstanceProvider
							instanceId="map-a"
							widgetId="gods-eye-view"
							componentId="map-a"
							actionBindings={{
								entityClicked: {
									workflow: { flowId: "handler-node", inputMappings: {} },
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

	/** Run one action and return the toasts it raised. */
	const run = async (probe: string, action: Action, componentId: string) => {
		toastError.mockClear();
		toastInfo.mockClear();
		await act(async () => {
			await controls[probe].executeAction(action, componentId);
		});
		return {
			errors: toastError.mock.calls as unknown as ToastCall[],
			infos: toastInfo.mock.calls as unknown as ToastCall[],
		};
	};

	/** Run one action and return the description of the error toast it raised. */
	const failureDescription = async (
		probe: string,
		action: Action,
		componentId: string,
	) => {
		const { errors, infos } = await run(probe, action, componentId);
		expect(errors).toHaveLength(1);
		expect(infos).toHaveLength(0);
		return errors[0][1]?.description;
	};

	return { run, failureDescription, consoleError, runs };
}

type ToastCall = [string, { description?: string } | undefined];

const WORKFLOW_ACTION: Action = {
	name: "workflow_event",
	context: {},
	pageAction,
};
const WIDGET_ACTION: Action = {
	name: "widget_event",
	context: { actionId: "entityClicked" },
};

const LONG_REASON = "x".repeat(500);

const REJECTIONS: [string, unknown, string | undefined][] = [
	[
		"a string",
		"  Profile and auth required for remote execution \n",
		"Profile and auth required for remote execution",
	],
	[
		"an object with a message",
		{ message: "Event not found: page-event" },
		"Event not found: page-event",
	],
	[
		"a serialized Tauri error",
		{ error: "Failed to fetch event data" },
		"Failed to fetch event data",
	],
	[
		"an Error",
		new Error("Governed Page action is missing its Event id."),
		"Governed Page action is missing its Event id.",
	],
	[
		"a hosted API error",
		new ApiResponseError({
			status: 500,
			message: "Event runtime unavailable",
			errorId: "req-123",
			path: "https://api.example.test/x?token=SECRET",
		}),
		"Event runtime unavailable",
	],
	["a long string", LONG_REASON, `${"x".repeat(299)}…`],
	["an empty string", "   ", undefined],
	["an object without a reason", { code: 7, message: 42 }, undefined],
	["an unknown value", 42, undefined],
];

describe("workflow start failure toasts", () => {
	test.each(REJECTIONS)(
		"a rejection with %s is described in both the workflow and widget toast",
		async (_label, rejection, expected) => {
			const page = await setupFailingPage(rejection);

			const workflow = await page.failureDescription(
				"page",
				WORKFLOW_ACTION,
				"page-button",
			);
			const widget = await page.failureDescription(
				"widget",
				WIDGET_ACTION,
				"map-a",
			);

			expect(workflow).toBe(expected ?? WORKFLOW_FALLBACK);
			expect(widget).toBe(expected ?? WIDGET_FALLBACK);
			if (expected) expect(expected.length).toBeLessThanOrEqual(300);
			// The live-page bridge gets the toast's reason, never "[object Object]".
			expect(
				page.runs.map((run) => [run.componentId, run.status, run.errorMessage]),
			).toEqual([
				["page-button", "error", expected ?? WORKFLOW_FALLBACK],
				["map-a", "error", expected ?? WIDGET_FALLBACK],
			]);
			// The rejection reaches the toast only, never the console.
			for (const call of page.consoleError.mock.calls) {
				expect(call).toHaveLength(1);
				expect(typeof call[0]).toBe("string");
			}
		},
	);
});

const STALE_ACTION = "The Page action is stale or invalid";

const CONTRACT_REJECTIONS: [string, unknown][] = [
	["a string", STALE_ACTION],
	["a serialized Tauri error", { error: STALE_ACTION }],
	["an Error", new Error(STALE_ACTION)],
	[
		"a hosted API error",
		new ApiResponseError({
			status: 409,
			message: "The Page manifest is stale",
			errorId: "req-9",
		}),
	],
];

describe("actions replayed from the surface cache", () => {
	test("a pending action waits for the load run instead of failing", async () => {
		const page = await setupFailingPage("never dispatched");
		const { errors, infos } = await page.run(
			"page",
			{ name: "workflow_event", context: {}, pendingPageAction: true },
			"page-button",
		);
		expect(errors).toHaveLength(0);
		expect(infos.map(([title]) => title)).toEqual([
			"This page is still loading — try that again in a moment.",
		]);
		expect(page.runs).toHaveLength(0);
	});
});

describe("Page contract failure toasts", () => {
	test.each(CONTRACT_REJECTIONS)(
		"a contract rejection with %s says the Page changed for both the workflow and widget action",
		async (_label, rejection) => {
			const page = await setupFailingPage(rejection);

			for (const [probe, action, componentId] of [
				["page", WORKFLOW_ACTION, "page-button"],
				["widget", WIDGET_ACTION, "map-a"],
			] as const) {
				const { errors, infos } = await page.run(probe, action, componentId);
				expect(errors).toHaveLength(0);
				expect(
					infos.map(([title, options]) => [title, options?.description]),
				).toEqual([
					[
						"This Page changed",
						"Refreshing it now — try that again in a moment.",
					],
				]);
			}
		},
	);
});
