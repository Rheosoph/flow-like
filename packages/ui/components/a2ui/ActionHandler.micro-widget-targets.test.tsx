import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { Action, MicroWidgetInstanceComponent } from "./types";

let root: Root | undefined;
const cleanup: (() => void)[] = [];

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	for (const restore of cleanup.reverse()) restore();
	cleanup.length = 0;
});

const APP_ID = "app-page";
const EVENT_ID = "event-page";
const SURFACE_ID = "page-map";
const SECRET_ELEMENT = `${SURFACE_ID}/secret-field`;

/** A widget payload carrying every target key a built-in action reads. */
const HOSTILE_PAYLOAD = {
	route: "/attacker",
	queryParams: { steal: "1" },
	url: "https://attacker.example/phish",
	appId: "attacker-app",
	eventId: "attacker-event",
	nodeId: "attacker-node",
	boardId: "attacker-board",
	actionId: "attackerAction",
	feedbackId: "attacker-feedback",
	commentComponentId: "secret-field",
	includeState: true,
	pageContextMode: "query",
	pageContextQueryParamAllowlist: "token",
	includePageHash: true,
	successMessage: "Attacker toast",
	rating: 1,
	comment: "widget comment",
};

const pageUrl = (params: Record<string, string>) =>
	`/use?${new URLSearchParams(params).toString()}`;

interface FeedbackWrite {
	appId: string;
	eventId: string;
	feedbackId: string;
	body: {
		rating: number;
		comment: string;
		globalState?: unknown;
		localState: {
			elementValues?: Record<string, unknown>;
			pageContext: Record<string, unknown> | null;
		};
	};
}

/** Mount a page with one widget instance and record every navigation target. */
async function setupPageHarness() {
	const window = new Window({ url: "https://local/use?token=secret#section" });
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
		{ ActionProvider, useExecuteAction, useSetElementValue },
		{ WidgetInstanceProvider },
		{ resolveMicroWidgetContractEvent },
		{ useBackendStore },
		{ appGlobalState, pageLocalState },
		uiState,
		{ AppRouterContext },
		{ PathnameContext },
		{ toast },
	] = await Promise.all([
		import("./ActionHandler"),
		import("./layout/A2UIWidgetInstance"),
		import("./layout/A2UIMicroWidget"),
		import("../../state/backend-state"),
		import("../../lib/idb-storage"),
		import("../../db/ui-state-db"),
		import("next/dist/shared/lib/app-router-context.shared-runtime"),
		import("next/dist/shared/lib/hooks-client-context.shared-runtime"),
		import("sonner"),
	]);
	const toastSuccess = spyOn(toast, "success").mockImplementation(() => "");
	const opened = spyOn(window, "open").mockImplementation(() => null);
	const spies = [
		spyOn(appGlobalState, "getAll").mockResolvedValue({}),
		spyOn(pageLocalState, "getAll").mockResolvedValue({}),
		spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
		spyOn(uiState.uiElementValues, "set").mockResolvedValue(undefined as never),
		spyOn(uiState, "pruneElementValues").mockResolvedValue(),
		spyOn(console, "log").mockImplementation(() => {}),
		spyOn(console, "warn").mockImplementation(() => {}),
		toastSuccess,
		opened,
	];
	cleanup.push(() => {
		for (const spy of spies) spy.mockRestore();
	});

	const feedback: FeedbackWrite[] = [];
	const previousBackend = useBackendStore.getState().backend;
	cleanup.push(() => useBackendStore.setState({ backend: previousBackend }));
	useBackendStore.getState().setBackend({
		boardState: {},
		eventState: {
			upsertEventFeedback: async (
				appId: string,
				eventId: string,
				feedbackId: string,
				body: FeedbackWrite["body"],
			) => {
				feedback.push({ appId, eventId, feedbackId, body });
			},
		},
	} as never);

	const navigations: string[] = [];
	const router = {
		push: (href: string) => navigations.push(href),
		replace: (href: string) => navigations.push(`replace:${href}`),
		prefetch: () => {},
		back: () => {},
		forward: () => {},
		refresh: () => {},
	};

	const controls: {
		executeAction?: ReturnType<typeof useExecuteAction>["executeAction"];
		setElementValue?: ReturnType<typeof useSetElementValue>;
	} = {};
	function Probe() {
		controls.executeAction = useExecuteAction().executeAction;
		controls.setElementValue = useSetElementValue();
		return null;
	}

	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);
	await act(async () => {
		root?.render(
			<AppRouterContext.Provider value={router as never}>
				<PathnameContext.Provider value="/use">
					<ActionProvider
						appId={APP_ID}
						eventId={EVENT_ID}
						surfaceId={SURFACE_ID}
						isPreviewMode
						components={{}}
					>
						<WidgetInstanceProvider
							instanceId="map-a"
							widgetId="gods-eye-view"
							componentId="map-a"
							actionBindings={{}}
							eventHandlers={{
								entityClicked: [
									{
										name: "navigate_page",
										context: { route: "/authored-handler" },
									},
								],
								attackerAction: [
									{
										name: "navigate_page",
										context: { route: "/attacker-handler" },
									},
								],
							}}
						>
							<Probe />
						</WidgetInstanceProvider>
					</ActionProvider>
				</PathnameContext.Provider>
			</AppRouterContext.Provider>,
		);
	});
	await act(async () => {
		controls.setElementValue?.(SECRET_ELEMENT, "page secret");
	});

	// Build the context exactly as the micro widget host does for an iframe event.
	const resolution = resolveMicroWidgetContractEvent(
		{
			id: "map-a",
			type: "microWidgetInstance",
			instanceId: "map-a",
			packageId: "com.example.globe",
			widgetId: "gods-eye-view",
			packageVersion: "1.0.0",
			contract: {
				contractVersion: 1,
				id: "gods-eye-view",
				events: { entityClicked: {} },
			},
		} as unknown as MicroWidgetInstanceComponent,
		{ name: "entityClicked", payload: HOSTILE_PAYLOAD },
	);
	if (resolution.kind !== "dispatch") {
		throw new Error("The micro widget event was not dispatched");
	}

	const executeAction: ReturnType<typeof useExecuteAction>["executeAction"] = (
		...args
	) => {
		if (!controls.executeAction) throw new Error("Probe did not mount");
		return controls.executeAction(...args);
	};

	return {
		executeAction,
		widgetContext: resolution.context,
		navigations,
		opened,
		feedback,
		toastSuccess,
	};
}

describe("micro widget action targets", () => {
	test("a micro widget payload cannot change the targets of the built-in actions it starts", async () => {
		const page = await setupPageHarness();
		expect(page.widgetContext.actionId).toBe("entityClicked");

		const run = (action: Action) =>
			page.executeAction(action, "map-a", page.widgetContext, {
				origin: "micro_widget",
			});
		await act(async () => {
			await run({
				name: "navigate_page",
				context: { route: "/authored", queryParams: { tab: "map" } },
			});
			await run({ name: "navigate_page", context: {} });
			await run({
				name: "external_link",
				context: { url: "https://authored.example/docs" },
			});
			await run({ name: "external_link", context: {} });
			await run({ name: "navigate_app_config", context: {} });
			await run({
				name: "navigate_app_config",
				context: { appId: "authored-app" },
			});
			await run({ name: "navigate_app_overview", context: {} });
			await run({
				name: "navigate_app_overview",
				context: { appId: "authored-app", eventId: "authored-event" },
			});
			await run({ name: "submit_feedback", context: { includeState: false } });
			await run({
				name: "submit_feedback",
				context: {
					appId: "authored-app",
					eventId: "authored-event",
					feedbackId: "authored-feedback",
					successMessage: "Authored thanks",
				},
			});
		});

		expect(page.navigations).toEqual([
			pageUrl({ id: APP_ID, route: "/authored", tab: "map" }),
			`/library/config?id=${APP_ID}`,
			"/library/config?id=authored-app",
			`/store?id=${APP_ID}&eventId=${EVENT_ID}`,
			"/store?id=authored-app&eventId=authored-event",
		]);
		expect(page.opened.mock.calls).toEqual([
			["https://authored.example/docs", "_blank", "noopener,noreferrer"],
		]);

		const pathOnlyContext = {
			pathname: "/use",
			routePathname: "/use",
			search: "",
			hash: "",
			queryParams: {},
		};
		expect(page.feedback).toHaveLength(2);
		const [inherited, authored] = page.feedback;
		expect(inherited.appId).toBe(APP_ID);
		expect(inherited.eventId).toBe(EVENT_ID);
		expect(inherited.feedbackId).toBe(`${SURFACE_ID}:map-a`);
		// Rating and comment are event data; the element read and the disclosed state are not.
		expect(inherited.body.rating).toBe(1);
		expect(inherited.body.comment).toBe("widget comment");
		expect(inherited.body.globalState).toBeUndefined();
		expect(inherited.body.localState.elementValues).toBeUndefined();
		expect(inherited.body.localState.pageContext).toEqual(pathOnlyContext);

		expect(authored.appId).toBe("authored-app");
		expect(authored.eventId).toBe("authored-event");
		expect(authored.feedbackId).toBe(`${SURFACE_ID}:authored-feedback`);
		expect(authored.body.comment).toBe("widget comment");
		expect(authored.body.localState.pageContext).toEqual(pathOnlyContext);

		expect(page.toastSuccess).toHaveBeenCalledTimes(2);
		expect(page.toastSuccess).not.toHaveBeenCalledWith("Attacker toast");
		expect(page.toastSuccess).toHaveBeenLastCalledWith("Authored thanks");
	});

	test("a widget_event keeps its host-set action id and passes the origin to routed actions", async () => {
		// The micro widget host provides only action bindings; these handlers guard the executeAction propagation itself. A2UIMicroWidget.dom.test.tsx covers the host wiring.
		const page = await setupPageHarness();

		await act(async () => {
			await page.executeAction(
				{ name: "widget_event", context: page.widgetContext },
				"map-a",
				undefined,
				{ origin: "micro_widget" },
			);
		});

		expect(page.navigations).toEqual([
			pageUrl({ id: APP_ID, route: "/authored-handler" }),
		]);
	});

	test("without an origin the event context still fills and overrides action targets", async () => {
		const page = await setupPageHarness();

		const run = (action: Action) =>
			page.executeAction(action, "page-button", HOSTILE_PAYLOAD);
		await act(async () => {
			await run({ name: "navigate_page", context: { route: "/authored" } });
			await run({
				name: "external_link",
				context: { url: "https://authored.example/docs" },
			});
			await run({ name: "navigate_app_config", context: {} });
			await run({
				name: "navigate_app_overview",
				context: { appId: "authored-app" },
			});
			await run({
				name: "submit_feedback",
				context: { feedbackId: "authored-feedback", includeState: false },
			});
			await page.executeAction(
				{ name: "widget_event", context: HOSTILE_PAYLOAD },
				"page-button",
			);
		});

		expect(page.navigations).toEqual([
			pageUrl({ id: APP_ID, route: "/attacker", steal: "1" }),
			"/library/config?id=attacker-app",
			"/store?id=attacker-app&eventId=attacker-event",
			pageUrl({ id: APP_ID, route: "/attacker", steal: "1" }),
		]);
		expect(page.opened.mock.calls).toEqual([
			["https://attacker.example/phish", "_blank", "noopener,noreferrer"],
		]);

		expect(page.feedback).toHaveLength(1);
		const [write] = page.feedback;
		expect(write.appId).toBe("attacker-app");
		expect(write.eventId).toBe("attacker-event");
		expect(write.feedbackId).toBe(`${SURFACE_ID}:attacker-feedback`);
		expect(write.body.comment).toBe("page secret");
		expect(write.body.globalState).toBeDefined();
		expect(write.body.localState.elementValues?.[SECRET_ELEMENT]).toBe(
			"page secret",
		);
		expect(write.body.localState.pageContext).toMatchObject({
			hash: "#section",
		});
		expect(page.toastSuccess).toHaveBeenLastCalledWith("Attacker toast");
	});
});
