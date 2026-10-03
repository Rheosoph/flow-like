import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ComponentProps } from "react";
import { act } from "react";
import type { IOAuthConsentStore } from "../../db/oauth-db";
import { EVENT_DEFINITIONS } from "../../lib/event-definitions";
import { withDeviceEventSource } from "../../lib/event-source";
import type { IOAuthTokenStoreWithPending } from "../../lib/oauth/types";
import {
	type IBoard,
	IExecutionMode,
	IExecutionStage,
	ILogLevel,
	type INode,
} from "../../lib/schema/flow/board";
import type { IBoardSummary } from "../../lib/schema/flow/board-summary";
import {
	type IEvent,
	IEventExecutionMode,
	IEventExposure,
} from "../../lib/schema/flow/event";
import type { IHub } from "../../lib/schema/hub/hub";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../lib/uint8";
import type { IBackendState } from "../../state/backend-state";
import type { IRouteMapping } from "../../state/backend-state/route-state";
import type { IEventMapping } from "../interfaces/interfaces";
import {
	byRole,
	click,
	installDom,
	queryByRole,
	settle,
	typeInto,
} from "../settings/devices/testing/dom-harness";

const dom = installDom();
const { EventForm } = await import("./event-form");
const { useBackendStore } = await import("../../state/backend-state");
const { mountDevices, cleanupDevices, preloadDevices } = await import(
	"../settings/devices/testing/mount-devices"
);
await preloadDevices();
const initialBackend = useBackendStore.getState().backend;
const clients: QueryClient[] = [];

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	for (const client of clients.splice(0)) client.clear();
	useBackendStore.setState({ backend: initialBackend });
});
afterAll(dom.restore);

const eventConfig: IEventMapping = Object.fromEntries(
	Object.entries(EVENT_DEFINITIONS).map(([key, definition]) => [
		key,
		{ ...definition, useInterfaces: {}, configInterfaces: {} },
	]),
);
eventConfig.events_simple = {
	...eventConfig.events_simple,
	configs: {
		...eventConfig.events_simple.configs,
		api: {
			sink_type: "http",
			method: "GET",
			path: "/orders",
			public_endpoint: false,
		},
	},
};

const nodes = {
	receive: {
		id: "receive",
		name: "events_simple",
		friendly_name: "Receive order",
		start: true,
	},
	refund: {
		id: "refund",
		name: "events_simple",
		friendly_name: "Refund approved",
		start: true,
	},
	chat: {
		id: "chat",
		name: "events_chat",
		friendly_name: "Order support",
		start: true,
	},
} as unknown as Record<string, INode>;

function board(mode = IExecutionMode.Hybrid, oauth = false): IBoard {
	return {
		id: "orders",
		name: "Order intake",
		execution_mode: mode,
		nodes: oauth
			? {
					...nodes,
					receive: {
						...nodes.receive,
						oauth_providers: ["erp"],
						required_oauth_scopes: { erp: ["orders.read"] },
					},
				}
			: nodes,
		layers: {},
		refs: {},
		variables: {},
	} as IBoard;
}

function summary(mode: IExecutionMode): IBoardSummary {
	return {
		id: "orders",
		name: "Order intake",
		description: "",
		executionMode: mode,
		stage: IExecutionStage.Dev,
		logLevel: ILogLevel.Info,
		version: [1, 0, 0],
		nodeCount: 3,
		connectionCount: 0,
		variableCount: 0,
		layerCount: 0,
		commentCount: 0,
		pages: [],
		entryNodes: Object.values(nodes).map((node) => ({
			nodeId: node.id,
			nodeType: node.name,
			friendlyName: node.friendly_name ?? undefined,
		})),
	};
}

function savedEvent(patch: Partial<IEvent> = {}): IEvent {
	return {
		id: "existing",
		name: "Receive order",
		description: "",
		board_id: "orders",
		node_id: "receive",
		event_type: "quick_action",
		config: [],
		variables: {},
		execution_mode: IEventExecutionMode.Local,
		exposure: IEventExposure.Public,
		...patch,
	} as IEvent;
}

async function flush() {
	for (let i = 0; i < 4; i++) await settle();
}

async function mount(
	options: {
		mode?: IExecutionMode;
		canExecuteLocally?: boolean;
		offline?: boolean;
		oauth?: boolean;
		getBoard?: IBackendState["boardState"]["getBoard"];
		routes?: () => Promise<IRouteMapping[]>;
		props?: Partial<ComponentProps<typeof EventForm>>;
	} = {},
) {
	const mode = options.mode ?? IExecutionMode.Hybrid;
	const boardReads: unknown[][] = [];
	useBackendStore.setState({
		backend: {
			capabilities: () => ({
				canExecuteLocally: options.canExecuteLocally ?? true,
			}),
			isOffline: async function isOffline() {
				return options.offline ?? false;
			},
			boardState: {
				getBoardSummaries: async function getBoardSummaries() {
					return [summary(mode)];
				},
				getBoard: async function getBoard(
					...args: Parameters<IBackendState["boardState"]["getBoard"]>
				) {
					boardReads.push(args);
					return options.getBoard
						? options.getBoard(...args)
						: board(mode, options.oauth);
				},
				getBoardVersions: async function getBoardVersions() {
					return [
						[1, 0, 0],
						[0, 9, 0],
					];
				},
			},
			pageState: {
				getPages: async function getPages() {
					return [
						{
							appId: "app",
							boardId: "orders",
							pageId: "desk",
							name: "Order desk",
						},
					];
				},
			},
			routeState: {
				getRoutes: async function getRoutes() {
					return options.routes ? options.routes() : [];
				},
			},
		} as unknown as IBackendState,
	});
	const client = new QueryClient({
		defaultOptions: {
			queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
		},
	});
	clients.push(client);
	const submissions: Parameters<
		ComponentProps<typeof EventForm>["onSubmit"]
	>[] = [];
	const view = await dom.render(
		<QueryClientProvider client={client}>
			<EventForm
				appId="app"
				eventConfig={eventConfig}
				onSubmit={(...args) => {
					submissions.push(args);
				}}
				onCancel={() => {}}
				{...options.props}
			/>
		</QueryClientProvider>,
	);
	await flush();
	return { ...view, client, submissions, boardReads };
}

const create = () => byRole("button", /^create event$/i) as HTMLButtonElement;
const destination = (name: "This computer" | "Hub") =>
	byRole("button", new RegExp(`^${name}`)) as HTMLButtonElement;
async function pick(name = "Receive order") {
	await click(byRole("radio", `Order intake: ${name}`));
	await flush();
}

describe("Event creation", () => {
	test("a template's device source preference is removed when creating on the hub", async () => {
		const view = await mount({
			props: {
				event: withDeviceEventSource(
					savedEvent({
						event_type: "api",
						config: convertJsonToUint8Array(
							eventConfig.events_simple.configs.api,
						),
					}),
				),
			},
		});
		await click(destination("Hub"));
		await click(create());
		expect(view.submissions).toHaveLength(1);
		expect(parseUint8ArrayToJson(view.submissions[0][0].config)).toEqual(
			eventConfig.events_simple.configs.api,
		);
		expect(view.submissions[0][0].execution_mode).toBe(
			IEventExecutionMode.Remote,
		);
	});

	test("a device creation retry accepts its reserved route after the first response is lost", async () => {
		let creates = 0;
		const appId = "app_visitor_checkin";
		const reservedId = "device-route-retry";
		const view = await mountDevices(
			({ overrides }) => (
				<EventForm
					appId={appId}
					eventConfig={eventConfig}
					deviceWorkspaceOverrides={overrides}
					deviceEventId={reservedId}
					event={savedEvent({
						event_type: "page",
						default_page_id: "desk",
						route: "/desk",
						board_version: [1, 0, 0],
					})}
					onSubmit={() => {
						throw new Error("Unexpected source creation");
					}}
					onCancel={() => {}}
					onCreateDevice={async () => {
						creates += 1;
						throw new Error("Save response lost");
					}}
				/>
			),
			{
				providers: false,
				backend: {
					boardState: {
						getBoardSummaries: async () => [summary(IExecutionMode.Hybrid)],
						getBoard: async () => board(),
						getBoardVersions: async () => [[1, 0, 0]],
					} as unknown as IBackendState["boardState"],
					pageState: {
						getPages: async () => [],
					} as unknown as IBackendState["pageState"],
					routeState: {
						getRoutes: async function getRoutes() {
							return creates ? [{ path: "/desk", eventId: reservedId }] : [];
						},
					} as unknown as IBackendState["routeState"],
				},
			},
		);
		await click(byRole("button", "Create only"));
		await view.settle();
		expect(creates).toBe(1);
		await act(async () => {
			await view.fake.queryClient.invalidateQueries({
				queryKey: ["getRoutes", appId],
			});
		});
		await view.settle();
		expect(view.fake.queryClient.getQueryData(["getRoutes", appId])).toEqual([
			{ path: "/desk", eventId: reservedId },
		]);
		await click(byRole("button", "Create only"));
		await view.settle();
		expect(creates).toBe(2);
		expect(view.container.textContent).not.toContain(
			"This path is already used by another route.",
		);
	});

	test("changing the start node resets its trigger type and configuration", async () => {
		const view = await mount();
		await pick();
		await click(byRole("button", "API endpoint"));
		await typeInto(byRole("textbox", "Path"), "/custom");
		await pick("Refund approved");
		expect(byRole("button", "Quick action").getAttribute("aria-pressed")).toBe(
			"true",
		);
		expect(queryByRole("textbox", "Path")).toBeNull();
		await click(byRole("button", "API endpoint"));
		expect((byRole("textbox", "Path") as HTMLInputElement).value).toBe(
			"/orders",
		);
		await click(create());
		expect(view.submissions[0]?.[0].node_id).toBe("refund");
		expect(
			parseUint8ArrayToJson(view.submissions[0]?.[0].config),
		).toMatchObject({ path: "/orders", method: "GET" });
	});

	test("serializes live API settings, trims the name and keeps the chosen flow version", async () => {
		const view = await mount({
			props: {
				event: savedEvent({
					event_type: "api",
					board_version: [0, 9, 0],
					config: convertJsonToUint8Array(
						eventConfig.events_simple.configs.api,
					),
				}),
			},
		});
		await typeInto(byRole("textbox", /event name/i), "  Import orders  ");
		await typeInto(byRole("combobox", "Method"), "PATCH");
		await typeInto(byRole("textbox", "Path"), "/incoming/orders");
		await click(create());
		expect(view.submissions).toHaveLength(1);
		expect(view.submissions[0][0]).toMatchObject({
			name: "Import orders",
			board_id: "orders",
			node_id: "receive",
			board_version: [0, 9, 0],
		});
		expect(parseUint8ArrayToJson(view.submissions[0][0].config)).toEqual({
			sink_type: "http",
			method: "PATCH",
			path: "/incoming/orders",
			public_endpoint: false,
		});
		expect(view.submissions[0][0]).not.toHaveProperty("target_kind");
		expect(view.boardReads).toContainEqual(["app", "orders", [0, 9, 0]]);
	});

	test.each([
		["events_chat", "Chat UI", "simple_chat"],
		["events_custom", "Default", "default"],
	])(
		"a pinned %s node requires a compatible trigger",
		async (nodeType, label, eventType) => {
			const view = await mount({
				getBoard: async (_appId, _boardId, version) => ({
					...board(),
					nodes: version
						? { ...nodes, receive: { ...nodes.receive, name: nodeType } }
						: nodes,
				}),
				props: { event: savedEvent() },
			});
			expect(create().disabled).toBe(false);
			await click(byRole("combobox", /flow version/i));
			await click(byRole("option", "v0.9.0"));
			await flush();
			expect(create().disabled).toBe(true);
			expect(view.container.textContent).toContain(
				"Choose a trigger supported by this flow version.",
			);
			await click(create());
			expect(view.submissions).toHaveLength(0);
			await click(byRole("button", label));
			await click(create());
			expect(view.submissions[0]?.[0]).toMatchObject({
				board_version: [0, 9, 0],
				event_type: eventType,
			});
		},
	);

	test("a page cannot submit until route availability has loaded", async () => {
		let resolveRoutes!: (routes: IRouteMapping[]) => void;
		const pending = new Promise<IRouteMapping[]>((resolve) => {
			resolveRoutes = resolve;
		});
		const view = await mount({ routes: () => pending });
		await pick("Order desk");
		expect(create().disabled).toBe(true);
		await click(create());
		expect(view.submissions).toEqual([]);
		await act(async () => resolveRoutes([]));
		await flush();
		expect(create().disabled).toBe(false);
		await typeInto(byRole("textbox", /route path/i), "desk?preview=true");
		await click(create());
		expect(view.submissions[0]?.[0]).toMatchObject({
			default_page_id: "desk",
			event_type: "page",
			path: "/desk",
			route: "/desk",
		});
		expect(view.submissions[0]?.[0].node_id).toBeUndefined();
	});

	test("a failed route check prevents creation and can be retried", async () => {
		let fail = true;
		const view = await mount({
			routes: async () => {
				if (fail) throw new Error("Route read failed");
				return [];
			},
		});
		await pick("Order desk");
		expect(create().disabled).toBe(true);
		await click(create());
		expect(view.submissions).toEqual([]);
		fail = false;
		await click(byRole("button", "Retry route availability check"));
		await flush();
		expect(create().disabled).toBe(false);
		await click(create());
		expect(view.submissions).toHaveLength(1);
	});

	test("rejects a page route that another event already uses", async () => {
		const view = await mount({
			routes: async () => [{ path: "/desk", eventId: "other" }],
		});
		await pick("Order desk");
		await typeInto(byRole("textbox", /route path/i), "desk");
		await click(create());
		expect(view.submissions).toEqual([]);
		expect(view.container.textContent).toContain(
			"This path is already used by another route.",
		);
	});

	test("server-only events switch to the hub and submit as Remote", async () => {
		const view = await mount();
		await pick("Order support");
		await click(byRole("button", "Teams Bot"));
		await flush();
		expect(destination("This computer").disabled).toBe(true);
		expect(destination("Hub").getAttribute("aria-pressed")).toBe("true");
		await click(create());
		expect(view.submissions[0]?.[0]).toMatchObject({
			event_type: "teams",
			execution_mode: IEventExecutionMode.Remote,
		});
	});

	test("Local flows disable the hub and retain Local execution", async () => {
		const view = await mount({ mode: IExecutionMode.Local });
		await pick();
		expect(destination("Hub").disabled).toBe(true);
		await click(create());
		expect(view.submissions[0]?.[0].execution_mode).toBe(
			IEventExecutionMode.Local,
		);
	});

	test("Remote flows disable this computer and require the hub destination", async () => {
		const view = await mount({ mode: IExecutionMode.Remote });
		await pick();
		expect(destination("This computer").disabled).toBe(true);
		expect(create().disabled).toBe(true);
		await click(destination("Hub"));
		await click(create());
		expect(view.submissions[0]?.[0].execution_mode).toBe(
			IEventExecutionMode.Remote,
		);
	});

	test("passes consented OAuth tokens through normal creation", async () => {
		const token = {
			providerId: "erp",
			access_token: "test-access",
			refresh_token: "test-refresh",
			expires_at: 1_900_000_000_000,
			scopes: ["orders.read"],
			storedAt: 0,
		};
		const tokenStore = {
			getToken: async () => token,
			isExpired: () => false,
		} as unknown as IOAuthTokenStoreWithPending;
		const consentStore = {
			getConsentedProviderIds: async () => new Set(["erp"]),
		} as unknown as IOAuthConsentStore;
		const hub = {
			oauth_providers: {
				erp: {
					name: "ERP",
					auth_url: "https://example.test/auth",
					token_url: "https://example.test/token",
					scopes: ["orders.read"],
					client_id: "test-client",
				},
			},
		} as unknown as IHub;
		const view = await mount({
			oauth: true,
			props: { tokenStore, consentStore, hub },
		});
		await pick();
		await click(create());
		expect(view.submissions).toHaveLength(1);
		expect(view.submissions[0][1]).toEqual({
			erp: {
				access_token: "test-access",
				refresh_token: "test-refresh",
				expires_at: 1_900_000_000,
				token_type: "Bearer",
			},
		});
		expect(queryByRole("dialog")).toBeNull();
	});
});
