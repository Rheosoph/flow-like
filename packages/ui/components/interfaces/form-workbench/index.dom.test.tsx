import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { ReactNode } from "react";
import type { AuthContextProps } from "react-oidc-context";
import type { IIntercomEvent } from "../../../lib/schema/events/intercom-event";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IBackendState } from "../../../state/backend-state";
import type { ExecutionServiceContextValue } from "../../../state/execution-service-context-value";
import {
	click,
	installWorkbenchDom,
	mountWorkbench,
	settle,
} from "./testing/dom";
import { DESKTOP_LAYOUT } from "./testing/layouts";

/*
 * Integration smoke (PLAN §12.14): the host props → session → shell chain with the real session
 * controller, reducer, engine and views. Only the backend and the execution service are fakes.
 */

const dom = installWorkbenchDom();
const { FormWorkbenchInterface, identityOf } = await import("./index");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { AppRouterContext } = await import(
	"next/dist/shared/lib/app-router-context.shared-runtime"
);
const { AuthContext } = await import("react-oidc-context");
const { BackendContext } = await import("../../../state/backend-state");
const { ExecutionEngineProviderComponent } = await import(
	"../../../state/execution-engine-context"
);
const { ExecutionServiceContext } = await import(
	"../../../state/execution-service-context-value"
);

afterEach(dom.cleanup);
afterAll(dom.restore);

const APP_ID = "app-support-copilot";

const stamp = { secs_since_epoch: 0, nanos_since_epoch: 0 };
const intercom = (event_id: string, event_type: string, payload: unknown) =>
	({ event_id, event_type, payload, timestamp: stamp }) as IIntercomEvent;

const zeroFieldEvent = {
	id: "event-triage-request",
	name: "Triage selected request",
	description: "Analyze the selected customer request and prepare a response.",
	node_id: "node-triage-request",
	board_id: "board",
	event_type: "quick_action",
	inputs: [],
	route: null,
} as unknown as IEvent;

const backend = {
	eventState: {
		alwaysRemote: true,
		getEvents: async () => [],
		cancelExecution: async () => {},
		prerunEvent: async () => ({
			board_id: "board",
			runtime_variables: [],
			oauth_requirements: [],
			requires_local_execution: false,
			execution_mode: "Hybrid",
			can_execute_locally: false,
		}),
	},
	helperState: { fileToUrl: async () => "data:," },
	userState: {
		getPricing: async () => {
			throw new Error("offline");
		},
	},
	routeState: { getRoutes: async () => [] },
} as unknown as IBackendState;

const signedOut = {
	isAuthenticated: false,
	isLoading: false,
	user: null,
} as unknown as AuthContextProps;

interface Started {
	readonly appId: string;
	readonly eventId: string;
}

function fakeExecution(started: Started[]) {
	const executeEvent: ExecutionServiceContextValue["executeEvent"] = async (
		appId,
		eventId,
		_payload,
		_stream,
		onEventId,
		cb,
	) => {
		started.push({ appId, eventId });
		onEventId?.("backend-run-1");
		cb?.([
			intercom("s1", "run_initiated", {}),
			intercom("r1", "generic_result", "Triaged"),
			intercom("c1", "completed", { status: "completed" }),
		]);
		return undefined;
	};
	return { executeEvent } as ExecutionServiceContextValue;
}

const router = {
	push: () => {},
	replace: () => {},
	back: () => {},
	forward: () => {},
	refresh: () => {},
	prefetch: () => {},
};

function Host(props: {
	readonly started: Started[];
	readonly children: ReactNode;
}) {
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	return (
		<AppRouterContext.Provider value={router as never}>
			<QueryClientProvider client={client}>
				<AuthContext.Provider value={signedOut}>
					<BackendContext.Provider value={backend}>
						<ExecutionServiceContext.Provider
							value={fakeExecution(props.started)}
						>
							<ExecutionEngineProviderComponent showRunningTasks={false}>
								{props.children}
							</ExecutionEngineProviderComponent>
						</ExecutionServiceContext.Provider>
					</BackendContext.Provider>
				</AuthContext.Provider>
			</QueryClientProvider>
		</AppRouterContext.Provider>
	);
}

async function until(check: () => boolean, label: string) {
	for (let attempt = 0; attempt < 100; attempt++) {
		if (check()) return;
		await settle();
	}
	throw new Error(`timed out waiting for ${label}`);
}

describe("identityOf", () => {
	const desktop = { eventState: { alwaysRemote: false } } as IBackendState;
	const web = { eventState: { alwaysRemote: true } } as IBackendState;
	const signedIn = {
		isAuthenticated: true,
		user: { profile: { sub: "sub-1" } },
	} as AuthContextProps;

	test("desktop: signed in, memory per profile", () => {
		expect(
			identityOf(
				"app",
				{ ...desktop, profile: { id: "p-1" } } as IBackendState,
				undefined,
			),
		).toEqual({
			signedIn: true,
			memoryScope: "profile:p-1",
		});
		expect(identityOf("app", desktop, undefined).memoryScope).toBe(
			"profile:local",
		);
	});

	test("web: per signed-in user, session memory when signed out", () => {
		expect(identityOf("app", web, signedIn)).toEqual({
			signedIn: true,
			memoryScope: "user:sub-1",
		});
		expect(identityOf("app", web, signedOut)).toEqual({
			signedIn: false,
			memoryScope: null,
		});
		expect(identityOf("app", web, undefined)).toEqual({
			signedIn: false,
			memoryScope: null,
		});
	});

	test("hosted and service pages are never signed in", () => {
		expect(identityOf("hosted", desktop, signedIn)).toEqual({
			signedIn: false,
			memoryScope: null,
		});
		expect(identityOf("service", web, signedIn)).toEqual({
			signedIn: false,
			memoryScope: null,
		});
	});
});

describe("FormWorkbenchInterface", () => {
	test("a form without fields renders idle and runs to Done", async () => {
		const started: Started[] = [];
		const view = await mountWorkbench(
			<Host started={started}>
				<FormWorkbenchInterface
					appId={APP_ID}
					event={zeroFieldEvent}
					config={{}}
				/>
			</Host>,
			{ layout: DESKTOP_LAYOUT },
		);
		const root = () =>
			view.container.querySelector("[data-fw-root]") as HTMLElement | null;
		await until(
			() => root()?.querySelector("[data-fw-hero]") != null,
			"the first-run card",
		);
		const hero = root()?.querySelector("[data-fw-hero]") as HTMLElement;
		expect(hero.textContent).toContain("Triage selected request");
		const run = hero.querySelector("[data-fw-focus=run]") as HTMLElement;
		expect(run).not.toBeNull();
		expect(root()?.textContent).not.toContain("Done");
		expect(started).toEqual([]);

		await click(run);
		await until(() => (root()?.textContent ?? "").includes("Done"), "Done");
		expect(started).toEqual([{ appId: APP_ID, eventId: zeroFieldEvent.id }]);
		expect(root()?.textContent).toContain("Triaged");
		await view.unmount();
		await settle();
	});
});
