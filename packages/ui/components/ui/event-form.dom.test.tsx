import { afterAll, afterEach, describe, expect, spyOn, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ComponentProps, ReactElement } from "react";
import { act, useState } from "react";
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
const panelRenders = { count: 0 };
const prepareModule = await import(
	"../settings/devices/deploy/use-deploy-prepare"
);
const realUseDeployPrepare = prepareModule.useDeployPrepare;
const prepareSpy = spyOn(prepareModule, "useDeployPrepare").mockImplementation(
	((...args: Parameters<typeof prepareModule.useDeployPrepare>) => {
		panelRenders.count += 1;
		return realUseDeployPrepare(...args);
	}) as typeof prepareModule.useDeployPrepare,
);
const { EventForm } = await import("./event-form");
const { Dialog, DialogContent, DialogDescription, DialogTitle } = await import(
	"./dialog"
);
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
afterAll(() => {
	prepareSpy.mockRestore();
	dom.restore();
});

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
		offline?: boolean | "error";
		oauth?: boolean;
		host?: (form: ReactElement) => ReactElement;
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
				if (options.offline === "error")
					throw new Error("Offline check failed");
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
	const form = (
		<EventForm
			appId="app"
			eventConfig={eventConfig}
			onSubmit={(...args) => {
				submissions.push(args);
			}}
			onCancel={() => {}}
			{...options.props}
		/>
	);
	const view = await dom.render(
		<QueryClientProvider client={client}>
			{options.host ? options.host(form) : form}
		</QueryClientProvider>,
	);
	await flush();
	return { ...view, client, submissions, boardReads };
}

const create = () => byRole("button", /^create event$/i) as HTMLButtonElement;
const destination = (name: "This computer" | "Hub" | "Device") =>
	byRole("radio", new RegExp(`^${name}`)) as HTMLButtonElement;
const trigger = (name: string) => byRole("radio", name) as HTMLButtonElement;
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
					event={withDeviceEventSource(
						savedEvent({
							event_type: "page",
							default_page_id: "desk",
							route: "/desk",
							board_version: [1, 0, 0],
						}),
					)}
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
		const cachedRoutes: unknown = view.fake.queryClient.getQueryData([
			"getRoutes",
			appId,
		]);
		expect(cachedRoutes).toEqual([{ path: "/desk", eventId: reservedId }]);
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
		await click(trigger("API endpoint"));
		await typeInto(byRole("textbox", "Path"), "/custom");
		await pick("Refund approved");
		expect(trigger("Quick action").getAttribute("aria-checked")).toBe("true");
		expect(queryByRole("textbox", "Path")).toBeNull();
		await click(trigger("API endpoint"));
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
		await click(byRole("combobox", "Method"));
		await click(byRole("option", "PATCH"));
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
			await click(trigger(label));
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
		await click(trigger("Teams Bot"));
		await flush();
		expect(destination("This computer").disabled).toBe(true);
		expect(destination("Hub").getAttribute("aria-checked")).toBe("true");
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

	test("Remote flows disable this computer and move the event to the hub", async () => {
		const view = await mount({ mode: IExecutionMode.Remote });
		await pick();
		expect(destination("This computer").disabled).toBe(true);
		expect(destination("Hub").getAttribute("aria-checked")).toBe("true");
		expect(create().disabled).toBe(false);
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

const text = () => dom.document.body.textContent ?? "";
const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
const configOf = (submission?: { config?: number[] }) =>
	parseUint8ArrayToJson(submission?.config ?? []);

const deviceAppId = "app_visitor_checkin";

async function mountDeviceForm(
	props: Partial<ComponentProps<typeof EventForm>> & {
		boardMode?: IExecutionMode;
	} = {},
) {
	const { boardMode, ...formProps } = props;
	const submissions: Partial<IEvent>[] = [];
	const view = await mountDevices(
		({ overrides }) => (
			<EventForm
				appId={deviceAppId}
				eventConfig={eventConfig}
				deviceWorkspaceOverrides={overrides}
				onSubmit={(event) => {
					submissions.push(event);
				}}
				onCancel={() => {}}
				onCreateDevice={async () => {
					throw new Error("Stop after the draft");
				}}
				{...formProps}
			/>
		),
		{
			providers: false,
			backend: {
				boardState: {
					getBoardSummaries: async () => [
						summary(boardMode ?? IExecutionMode.Hybrid),
					],
					getBoard: async () => board(boardMode ?? IExecutionMode.Hybrid),
					getBoardVersions: async () => [[1, 0, 0]],
				} as unknown as IBackendState["boardState"],
				pageState: {
					getPages: async () => [],
				} as unknown as IBackendState["pageState"],
				routeState: {
					getRoutes: async () => [],
				} as unknown as IBackendState["routeState"],
			},
		},
	);
	return { view, submissions };
}

describe("Destination reasons", () => {
	test("every disabled destination says why under its title", async () => {
		await mount({ mode: IExecutionMode.Local });
		await pick();
		expect(destination("Hub").disabled).toBe(true);
		expect(destination("Hub").textContent).toContain(
			"This flow only runs locally.",
		);
		expect(destination("Device").disabled).toBe(true);
		expect(destination("Device").textContent).toContain(
			"Devices can't be set up from here.",
		);
	});

	test("a Remote flow and a web client explain why this computer is off", async () => {
		await mount({ mode: IExecutionMode.Remote });
		await pick();
		expect(destination("This computer").textContent).toContain(
			"This flow only runs on the server.",
		);
		await dom.cleanup();
		await mount({ canExecuteLocally: false });
		await pick();
		expect(destination("This computer").textContent).toContain(
			"Needs the desktop app.",
		);
	});

	test("an offline app explains why the hub is off", async () => {
		await mount({ offline: true });
		await pick();
		expect(destination("Hub").disabled).toBe(true);
		expect(destination("Hub").textContent).toContain("This app is offline.");
	});

	test("a failed offline check leaves the hub usable with a hint", async () => {
		const view = await mount({ canExecuteLocally: false, offline: "error" });
		await pick();
		expect(destination("Hub").disabled).toBe(false);
		expect(destination("Hub").getAttribute("aria-checked")).toBe("true");
		expect(destination("Hub").textContent).toContain(
			"Couldn't check the connection.",
		);
		await click(create());
		expect(view.submissions[0]?.[0].execution_mode).toBe(
			IEventExecutionMode.Remote,
		);
	});

	test("server-only types name why the other destinations are off", async () => {
		await mount();
		await pick("Order support");
		await click(trigger("Teams Bot"));
		await flush();
		expect(destination("This computer").textContent).toContain(
			"This trigger runs on the hub.",
		);
		expect(destination("Device").textContent).toContain(
			"Devices can't be set up from here.",
		);
	});
});

describe("Trigger types by destination", () => {
	test("types that fit no destination are disabled with their reason", async () => {
		await mount({ canExecuteLocally: false });
		await pick();
		expect((trigger("Deep link") as HTMLButtonElement).disabled).toBe(true);
		expect((trigger("Background service") as HTMLButtonElement).disabled).toBe(
			true,
		);
		expect((trigger("API endpoint") as HTMLButtonElement).disabled).toBe(false);
		expect(text()).toContain(
			"Deep link: This trigger only runs in the desktop app.",
		);
	});

	test("a hub that lacks a server-only sink disables that type", async () => {
		await mount({
			props: { hub: { supported_sinks: { http: true } } as unknown as IHub },
		});
		await pick("Order support");
		expect((trigger("Teams Bot") as HTMLButtonElement).disabled).toBe(true);
		expect(text()).toContain("This hub doesn't support teams.");
	});

	test("picking a type this destination cannot run moves the event and says so", async () => {
		const view = await mount({ mode: IExecutionMode.Remote });
		await pick();
		await click(trigger("REST server"));
		await flush();
		expect(destination("Hub").getAttribute("aria-checked")).toBe("true");
		await click(create());
		expect(view.submissions[0]?.[0]).toMatchObject({
			event_type: "rest",
			execution_mode: IEventExecutionMode.Remote,
		});
	});
});

describe("Automatic destination", () => {
	test("a trigger devices cannot run falls back from the device and says so", async () => {
		const { view, submissions } = await mountDeviceForm();
		await pick();
		expect(destination("Device").getAttribute("aria-checked")).toBe("true");
		await click(trigger("Deep link"));
		await view.settle();
		expect(destination("This computer").getAttribute("aria-checked")).toBe(
			"true",
		);
		expect(destination("Device").disabled).toBe(true);
		expect(destination("Device").textContent).toContain(
			"This trigger can't run on devices.",
		);
		expect(view.container.textContent).toContain(
			"This trigger can't run on devices. It was set to this computer.",
		);
		await click(create());
		await view.settle();
		expect(submissions[0]).toMatchObject({
			event_type: "deeplink",
			execution_mode: IEventExecutionMode.Local,
		});
		await click(trigger("Quick action"));
		await view.settle();
		expect(destination("Device").getAttribute("aria-checked")).toBe("true");
	});

	test("clicking the fallen-back destination pins it against a later device-capable trigger", async () => {
		const { view } = await mountDeviceForm();
		await pick();
		await click(trigger("Deep link"));
		await view.settle();
		expect(destination("This computer").getAttribute("aria-checked")).toBe(
			"true",
		);
		await click(destination("This computer"));
		await click(trigger("Quick action"));
		await view.settle();
		expect(destination("This computer").getAttribute("aria-checked")).toBe(
			"true",
		);
		expect(destination("Device").getAttribute("aria-checked")).toBe("false");
	});

	test("a template's Remote mode opens on the hub and Local on this computer", async () => {
		await mount({
			props: {
				event: savedEvent({ execution_mode: IEventExecutionMode.Remote }),
			},
		});
		expect(destination("Hub").getAttribute("aria-checked")).toBe("true");
		await dom.cleanup();
		await mount({ props: { event: savedEvent() } });
		expect(destination("This computer").getAttribute("aria-checked")).toBe(
			"true",
		);
	});

	test("a template's device source opens on a device", async () => {
		const { view } = await mountDeviceForm({
			event: withDeviceEventSource(savedEvent()),
		});
		expect(destination("Device").getAttribute("aria-checked")).toBe("true");
		expect(view.container.textContent).toContain("Create only");
	});
});

describe("Event name", () => {
	const name = () => byRole("textbox", /event name/i) as HTMLInputElement;

	test("follows the picked start until the name is edited", async () => {
		await mount();
		await pick();
		expect(name().value).toBe("Receive order");
		await pick("Refund approved");
		expect(name().value).toBe("Refund approved");
		await typeInto(name(), "Mine");
		await pick();
		expect(name().value).toBe("Mine");
		await typeInto(name(), "");
		await pick("Refund approved");
		expect(name().value).toBe("Refund approved");
	});

	test("a disabled Create points at the reason it is disabled", async () => {
		await mount();
		await pick();
		await typeInto(name(), "");
		expect(create().disabled).toBe(true);
		const hintId = create().getAttribute("aria-describedby");
		expect(hintId).toBeTruthy();
		expect(dom.document.getElementById(hintId ?? "")?.textContent).toContain(
			"Give the event a name",
		);
		await typeInto(name(), "Named");
		expect(create().disabled).toBe(false);
		expect(create().getAttribute("aria-describedby")).toBeNull();
	});

	test("the device destination states a missing name in the footer too", async () => {
		const { view } = await mountDeviceForm();
		await pick();
		expect(view.container.textContent).not.toContain("Give the event a name");
		await typeInto(name(), "");
		expect(view.container.textContent).toContain("Give the event a name");
	});

	test("a template's name is never replaced by a pick", async () => {
		await mount({ props: { event: savedEvent({ name: "From the lesson" }) } });
		await pick("Refund approved");
		expect(name().value).toBe("From the lesson");
	});
});

describe("Schedule settings", () => {
	test("a new schedule starts in the browser zone and runs where its destination runs", async () => {
		const view = await mount();
		await pick();
		await click(trigger("Schedule"));
		await click(create());
		expect(configOf(view.submissions[0]?.[0])).toMatchObject({
			sink_type: "cron",
			timezone,
			sink_execution: "LOCAL",
		});
		await click(destination("Hub"));
		await click(create());
		expect(configOf(view.submissions[1]?.[0])).toMatchObject({
			timezone,
			sink_execution: "REMOTE",
		});
	});

	test("a template's execution target never reaches a device draft", async () => {
		const drafts: Partial<IEvent>[] = [];
		const { view } = await mountDeviceForm({
			event: withDeviceEventSource(
				savedEvent({
					event_type: "cron",
					config: convertJsonToUint8Array({
						sink_type: "cron",
						expression: "0 9 * * *",
						sink_execution: "REMOTE",
					}),
				}),
			),
			onCreateDevice: async (draft) => {
				drafts.push(draft);
				throw new Error("Stop after the draft");
			},
		});
		await click(byRole("button", "Create only"));
		await view.settle();
		expect(drafts).toHaveLength(1);
		expect(configOf(drafts[0])).toEqual({
			sink_type: "cron",
			expression: "0 9 * * *",
		});
		expect(drafts[0]?.execution_mode).toBe(IEventExecutionMode.Local);
	});
});

describe("Trigger settings", () => {
	test("the settings editor starts fresh for every start node", async () => {
		let mounts = 0;
		function Probe() {
			const [count] = useState(() => ++mounts);
			return <output data-testid="probe">editor {count}</output>;
		}
		const cronFirst: IEventMapping = {
			...eventConfig,
			events_simple: {
				...eventConfig.events_simple,
				defaultEventType: "cron",
				configInterfaces: { cron: Probe as never },
			},
		};
		await mount({ props: { eventConfig: cronFirst } });
		await pick();
		expect(text()).toContain("editor 1");
		await pick("Refund approved");
		expect(text()).toContain("editor 2");
	});
});

describe("Device creation marker", () => {
	test("a template carrying the device marker creates on this computer without it", async () => {
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
		expect(destination("This computer").getAttribute("aria-checked")).toBe(
			"true",
		);
		await click(create());
		expect(configOf(view.submissions[0]?.[0])).toEqual(
			eventConfig.events_simple.configs.api,
		);
		expect(view.submissions[0]?.[0].execution_mode).toBe(
			IEventExecutionMode.Local,
		);
	});

	test("device creation hands over a draft without the marker", async () => {
		const drafts: Partial<IEvent>[] = [];
		const { view } = await mountDeviceForm({
			event: withDeviceEventSource(
				savedEvent({
					event_type: "api",
					config: convertJsonToUint8Array(
						eventConfig.events_simple.configs.api,
					),
				}),
			),
			onCreateDevice: async (draft) => {
				drafts.push(draft);
				throw new Error("Stop after the draft");
			},
		});
		await click(byRole("button", "Create only"));
		await view.settle();
		expect(configOf(drafts[0])).toEqual(eventConfig.events_simple.configs.api);
	});
});

describe("Deployment panel renders", () => {
	test("typing the name does not re-render the panel", async () => {
		const { view } = await mountDeviceForm();
		await pick();
		const name = byRole("textbox", /event name/i) as HTMLInputElement;
		await typeInto(name, "Mine");
		await view.settle();
		const before = panelRenders.count;
		for (const value of ["Mi", "Min", "Mine!", "Mine, renamed"])
			await typeInto(name, value);
		await view.settle();
		expect(panelRenders.count - before).toBe(0);
	});

	test("changing the start still reaches the panel", async () => {
		const { view } = await mountDeviceForm();
		await pick();
		await view.settle();
		const before = panelRenders.count;
		await pick("Refund approved");
		await view.settle();
		expect(panelRenders.count).toBeGreaterThan(before);
	});
});

describe("Saved device event", () => {
	test("Create only completes without a saved report and unmount clears it", async () => {
		const saved: (IEvent | null)[] = [];
		const completed: string[] = [];
		const { view } = await mountDeviceForm({
			onSavedChange: (event) => saved.push(event),
			onDeploymentComplete: (event) => completed.push(event.id),
			onCreateDevice: async (draft) => savedEvent({ ...draft, id: "device-1" }),
			event: withDeviceEventSource(savedEvent()),
		});
		await click(byRole("button", "Create only"));
		await view.settle();
		expect(completed).toEqual(["device-1"]);
		expect(saved.every((event) => event === null)).toBe(true);
		await view.unmount();
		expect(saved.at(-1)).toBeNull();
	});
});

describe("Deployed report", () => {
	test("passes the deployed state to the host, false while nothing is deployed", async () => {
		const reports: boolean[] = [];
		await mountDeviceForm({
			event: withDeviceEventSource(savedEvent()),
			onDeployedChange: (deployed) => reports.push(deployed),
		});
		expect(reports.length).toBeGreaterThan(0);
		expect(reports.every((deployed) => !deployed)).toBe(true);
	});
});

describe("OAuth consent inside the dialog", () => {
	const provider = {
		erp: {
			name: "ERP",
			auth_url: "https://example.test/auth",
			token_url: "https://example.test/token",
			scopes: ["orders.read"],
			client_id: "test-client",
		},
	};
	const token = {
		providerId: "erp",
		access_token: "test-access",
		refresh_token: "test-refresh",
		expires_at: 1_900_000_000_000,
		scopes: ["orders.read"],
		storedAt: 0,
	};
	const oauthProps = (consents: unknown[][] = []) => ({
		tokenStore: {
			getToken: async () => token,
			isExpired: () => false,
		} as unknown as IOAuthTokenStoreWithPending,
		consentStore: {
			getConsentedProviderIds: async () => new Set<string>(),
			setConsent: async (...args: unknown[]) => {
				consents.push(args);
			},
		} as unknown as IOAuthConsentStore,
		hub: { oauth_providers: provider } as unknown as IHub,
	});
	const acknowledge = () =>
		click(byRole("checkbox", /I understand and want to proceed/));

	test("a flow without consent asks first and submits the tokens once confirmed", async () => {
		const consents: unknown[][] = [];
		const view = await mount({ oauth: true, props: oauthProps(consents) });
		await pick();
		await click(create());
		await flush();
		expect(view.submissions).toHaveLength(0);
		expect(queryByRole("dialog")).not.toBeNull();
		await acknowledge();
		await click(byRole("button", "Continue"));
		await flush();
		expect(consents).toHaveLength(1);
		expect(view.submissions).toHaveLength(1);
		expect(view.submissions[0][1]).toMatchObject({
			erp: { access_token: "test-access" },
		});
		expect(queryByRole("dialog")).toBeNull();
	});

	test("cancelling the consent submits nothing and can be started again", async () => {
		const view = await mount({ oauth: true, props: oauthProps() });
		await pick();
		await click(create());
		await flush();
		await click(byRole("button", "Cancel", byRole("dialog")));
		await flush();
		expect(view.submissions).toHaveLength(0);
		expect(queryByRole("dialog")).toBeNull();
		await click(create());
		await flush();
		expect(queryByRole("dialog")).not.toBeNull();
	});

	test("closing the dialog together with the consent leaves the page clickable", async () => {
		let close = () => {};
		function Host({ children }: Readonly<{ children: ReactElement }>) {
			const [open, setOpen] = useState(true);
			close = () => setOpen(false);
			return (
				<Dialog open={open} onOpenChange={setOpen}>
					<DialogContent>
						<DialogTitle>New event</DialogTitle>
						<DialogDescription>Choose</DialogDescription>
						{children}
					</DialogContent>
				</Dialog>
			);
		}
		await mount({
			oauth: true,
			host: (form) => <Host>{form}</Host>,
			props: { ...oauthProps(), onSubmit: () => close() },
		});
		await pick();
		await click(create());
		await flush();
		expect(dom.document.body.style.pointerEvents).toBe("none");
		await acknowledge();
		await click(byRole("button", "Continue"));
		await flush();
		expect(queryByRole("dialog")).toBeNull();
		expect(dom.document.body.style.pointerEvents).not.toBe("none");
	});
});
