import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { QueryClient } from "@tanstack/react-query";
import { AppRouterContext } from "next/dist/shared/lib/app-router-context.shared-runtime";
import { SearchParamsContext } from "next/dist/shared/lib/hooks-client-context.shared-runtime";
import { type ReactNode, createContext, useContext } from "react";
import type { IEvent } from "../../../../lib/schema/flow/event";
import {
	type Inventory,
	createServiceBackend,
} from "../../../../lib/service-runtime/backend";
import type { RuntimeSession } from "../../../../lib/service-runtime/session";
import {
	useClientSearchParams,
	useSetQueryParams,
} from "../../../../lib/set-query-params";
import { useBackend, useBackendStore } from "../../../../state/backend-state";
import {
	ExecutionServiceContext,
	useExecutionService,
} from "../../../../state/execution-service-context-value";
import { byRole, click, installDom } from "../testing/dom-harness";

const dom = installDom({
	url: "https://app.test/settings/devices?sessionId=studio-history",
});
const requests: string[] = [];
const scopes: string[] = [];
let observedBackend: ReturnType<typeof useBackend>;
let observedQuery = "";
let observedConfig: Record<string, unknown> | undefined;

function InterfaceProbe({
	appId,
	event,
	onNavigate,
	config,
}: {
	appId: string;
	event: IEvent;
	onNavigate?: (route: string, replace: boolean) => void;
	config?: Record<string, unknown>;
}) {
	observedBackend = useBackend();
	const execution = useExecutionService();
	observedQuery = useClientSearchParams().toString();
	observedConfig = config;
	const setQuery = useSetQueryParams();
	return (
		<div>
			<span>
				{event.event_type}:{appId}
			</span>
			<button
				type="button"
				onClick={() =>
					void execution.executeEvent(appId, event.id, {
						id: "run",
						payload: { text: "hello" },
					})
				}
			>
				Run event
			</button>
			<button
				type="button"
				onClick={() => setQuery("sessionId", "device-chat")}
			>
				Choose chat
			</button>
			<button type="button" onClick={() => onNavigate?.("/form", false)}>
				Go to form
			</button>
		</div>
	);
}
mock.module("../../../interfaces/chat-default", () => ({
	ChatInterface: InterfaceProbe,
}));
mock.module("../../../interfaces/chat-default/message", () => ({
	ChatFeedbackEnabledContext: createContext(true),
}));
mock.module("../../../interfaces/form-workbench", () => ({
	FormWorkbenchInterface: InterfaceProbe,
}));
mock.module("../../../interfaces/page-interface", () => ({
	PageInterface: InterfaceProbe,
}));
mock.module("../../../../state/execution-engine-context", () => ({
	ExecutionEngineProviderComponent: ({
		children,
		executionScope,
	}: { children: ReactNode; executionScope: string }) => {
		scopes.push(executionScope);
		expect(useContext(ExecutionServiceContext)).toBeDefined();
		return children;
	},
}));
const { RuntimeView } = await import("./runtime-view");
const hostNavigation = mock(() => {});
const router = {
	push: hostNavigation,
	replace: hostNavigation,
	back: hostNavigation,
	forward: hostNavigation,
	refresh: hostNavigation,
	prefetch: hostNavigation,
	experimental_gesturePush: hostNavigation,
	bfcacheId: "studio",
};

function session(kind = "simple_chat"): RuntimeSession {
	const event = {
		id: "event",
		name: "Chat",
		event_type: kind,
		node_id: "",
		board_id: "board",
		event_version: [1, 0, 0],
		board_version: [1, 0, 0],
		config: [],
		route: "/chat",
		...(kind === "page" ? { default_page_id: "page" } : {}),
	} as unknown as IEvent;
	const form = {
		...event,
		id: "form",
		name: "Form",
		event_type: "generic_form",
		default_page_id: undefined,
		route: "/form",
	};
	const inventory: Inventory = { project_id: "project", events: [event, form] };
	const appId = `device-runtime:${crypto.randomUUID()}`;
	const controller = new AbortController();
	const request = async (path: string) => {
		requests.push(path);
		if (path.endsWith("/bootstrap"))
			return Response.json({
				project_id: "project",
				event_id: "event",
				event,
				page: { id: "page", components: [] },
				execution_revision: "revision",
				element_demand: { selectors: [], dynamic: false },
			});
		return new Response('event: done\ndata: {"completed":true}\n\n');
	};
	const { backend, bootstrap } = createServiceBackend(
		inventory,
		request,
		controller.signal,
		{ visibleAppId: appId },
	);
	const unavailable = async () => {
		throw new Error("Unsupported");
	};
	return {
		inventory,
		appId,
		backend,
		bootstrap,
		signal: controller.signal,
		close: () => controller.abort(),
		resolveResource: async (path) => path,
		client: new QueryClient(),
		execution: {
			executeBoard: unavailable,
			executeBoardDirect: unavailable,
			executeBoardRemote: unavailable,
			executeEvent: (...args) => backend.eventState.executeEvent(...args),
			executeEventDirect: (...args) => backend.eventState.executeEvent(...args),
		},
	};
}
async function mount(runtime: RuntimeSession) {
	return dom.render(
		<AppRouterContext.Provider value={router}>
			<SearchParamsContext.Provider
				value={new URLSearchParams("sessionId=studio-history")}
			>
				<RuntimeView session={runtime} />
			</SearchParamsContext.Provider>
		</AppRouterContext.Provider>,
	);
}
afterEach(async () => {
	await dom.cleanup();
	requests.length = 0;
	scopes.length = 0;
	hostNavigation.mockClear();
});
afterAll(() => {
	mock.restore();
	dom.restore();
});

test("Chat and form runs stay on the deployed backend without replacing Studio's backend or URL", async () => {
	const runtime = session();
	const originalBackend = useBackendStore.getState().backend;
	const view = await mount(runtime);
	expect(observedBackend).toBe(runtime.backend);
	expect(observedQuery).not.toContain("studio-history");
	expect(observedConfig?.attach_widget_snapshots).toBe(false);
	expect(useBackendStore.getState().backend).toBe(originalBackend);
	expect(scopes.every((scope) => scope === runtime.appId)).toBe(true);
	await click(byRole("button", "Choose chat", view.container));
	expect(observedQuery).toContain("sessionId=device-chat");
	expect(window.location.search).toBe("?sessionId=studio-history");
	await click(byRole("button", "Run event", view.container));
	expect(requests).toEqual(["/chat/event"]);
	await click(byRole("button", "Go to form", view.container));
	expect(view.container.textContent).toContain("generic_form:");
	await click(byRole("button", "Run event", view.container));
	expect(requests).toEqual(["/chat/event", "/run/form"]);
	expect(hostNavigation).not.toHaveBeenCalled();
	expect(useBackendStore.getState().backend).toBe(originalBackend);
});

test("Page bootstrap is loaded through the scoped backend and different opens use different cache identities", async () => {
	const first = session("page");
	const second = session("page");
	expect(first.appId).not.toBe(second.appId);
	const view = await mount(first);
	expect(requests).toEqual(["/pages/event/bootstrap"]);
	expect(view.container.textContent).toContain(`page:${first.appId}`);
	expect(observedBackend).toBe(first.backend);
	await view.unmount();
	const next = await mount(second);
	expect(next.container.textContent).toContain(`page:${second.appId}`);
	expect(observedQuery).not.toContain("device-chat");
});
