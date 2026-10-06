import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { ReactNode } from "react";
import type { AuthContextProps } from "react-oidc-context";
import type { IEvent } from "../../../../lib/schema/flow/event";
import type { IBackendState } from "../../../../state/backend-state";
import type { HostCapabilities, NavigateIntent } from "../contracts";
import { installWorkbenchDom, settle } from "../testing/dom";
import { FIXTURE_APP_HOST, FIXTURE_VIEWER } from "../testing/fixtures";
import type { FormSessionController } from "./controller";

/*
 * Two mounts of the same form at once (same app, event and partition): the second gets a controller of its
 * own, and the first keeps its own navigate and presentation (PLAN §3.7).
 */

const dom = installWorkbenchDom();
const { useFormSession, partitionOf } = await import("./use-form-session");
const { getSessionController } = await import("./registry");
const { AuthContext } = await import("react-oidc-context");
const { BackendContext } = await import("../../../../state/backend-state");
const { ExecutionEngineProviderComponent, useExecutionEngine } = await import(
	"../../../../state/execution-engine-context"
);

afterEach(dom.cleanup);
afterAll(dom.restore);

const APP_ID = "app-support-copilot";

const event = {
	id: "event-triage-request",
	name: "Triage selected request",
	description: "Analyze the selected customer request.",
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

const hostOf = (presentation: HostCapabilities["presentation"]) =>
	({
		...FIXTURE_APP_HOST,
		presentation,
		persistence: "session",
		memoryScope: null,
	}) satisfies HostCapabilities;

const PAGE = hostOf("page");
const TILE = hostOf("tile");
/** One object, as a host passes it: a fresh literal per render would rebuild the first mount's form and hide the bug. */
const CONFIG = {};

let engineSeen: object | null = null;

function Probe({
	id,
	host,
	navigate,
}: Readonly<{
	id: string;
	host: HostCapabilities;
	navigate: (intent: NavigateIntent) => void;
}>) {
	engineSeen = useExecutionEngine();
	const { state } = useFormSession({
		appId: APP_ID,
		event,
		config: CONFIG,
		host,
		viewer: FIXTURE_VIEWER,
		navigate,
	});
	return (
		<output data-mount={id} data-presentation={state.form.host.presentation} />
	);
}

const signedOut = {
	isAuthenticated: false,
	isLoading: false,
	user: null,
} as unknown as AuthContextProps;

function Providers({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<AuthContext.Provider value={signedOut}>
			<BackendContext.Provider value={backend}>
				<ExecutionEngineProviderComponent showRunningTasks={false}>
					{children}
				</ExecutionEngineProviderComponent>
			</BackendContext.Provider>
		</AuthContext.Provider>
	);
}

const presentationOf = (container: HTMLElement, id: string) =>
	container
		.querySelector(`[data-mount="${id}"]`)
		?.getAttribute("data-presentation");

describe("a second concurrent mount of the same form", () => {
	test("gets its own controller; the first keeps its navigate and its presentation", async () => {
		const navigatePage = (_intent: NavigateIntent) => {};
		const navigateTile = (_intent: NavigateIntent) => {};
		const page = <Probe id="page" host={PAGE} navigate={navigatePage} />;
		const view = await dom.render(<Providers>{page}</Providers>);
		await settle();
		expect(presentationOf(view.container, "page")).toBe("page");

		await view.rerender(
			<Providers>
				{page}
				<Probe id="tile" host={TILE} navigate={navigateTile} />
			</Providers>,
		);
		await settle();
		await settle();
		expect(presentationOf(view.container, "tile")).toBe("tile");
		expect(presentationOf(view.container, "page")).toBe("page");

		if (!engineSeen) throw new Error("no engine");
		const shared = getSessionController<FormSessionController>(
			engineSeen,
			APP_ID,
			event.id,
			() => {
				throw new Error("the shared controller should exist");
			},
			partitionOf(PAGE),
		);
		expect(shared.getState().form.host.presentation).toBe("page");
		const held = shared as unknown as { readonly navigateTo: unknown };
		expect(held.navigateTo === navigatePage).toBe(true);
		await view.unmount();
		await settle();
	});
});
