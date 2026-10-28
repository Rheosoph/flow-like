import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { type ReactNode, useMemo } from "react";
import { withDeviceEventSource } from "../../../lib/event-source";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IEventSinkStatus } from "../../../state/backend-state/event-state";
import {
	advance,
	byRole,
	click,
	installDom,
	queryByRole,
} from "../devices/testing/dom-harness";

const dom = installDom();
const { toast } = await import("sonner");
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../devices/testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import(
	"../devices/testing/fake-workspace"
);
const { AppRouterContext } = await import(
	"next/dist/shared/lib/app-router-context.shared-runtime"
);
const { SearchParamsContext } = await import(
	"next/dist/shared/lib/hooks-client-context.shared-runtime"
);
const EventsPage = (await import("./events-page")).default;

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const APP = "app_invoice_ai";
const at = { secs_since_epoch: 0, nanos_since_epoch: 0 };
const baseEvent = {
	id: "evt_dev",
	name: "Nightly digest",
	description: "",
	active: true,
	board_id: "board_0",
	node_id: "node_cron",
	event_type: "cron",
	event_version: [1, 0, 0],
	board_version: [1, 0, 0],
	priority: 0,
	execution_mode: "Local",
	exposure: "Public",
	config: [
		...new TextEncoder().encode(JSON.stringify({ expression: "0 * * * *" })),
	],
	created_at: at,
	updated_at: at,
	variables: {},
} as unknown as IEvent;
const MAPPING = {
	events_cron: {
		withSink: ["cron"],
		eventTypes: ["cron"],
		defaultEventType: "cron",
		configs: {},
		useInterfaces: {},
		configInterfaces: {},
	},
} as never;

function RouterFrame({
	pushes,
	children,
}: Readonly<{ pushes: string[]; children: ReactNode }>) {
	const router = useMemo(
		() => ({
			push: (href: string) => pushes.push(href),
			replace: () => undefined,
			back: () => undefined,
			forward: () => undefined,
			refresh: () => undefined,
			prefetch: () => undefined,
		}),
		[pushes],
	);
	return (
		<AppRouterContext.Provider value={router as never}>
			<SearchParamsContext.Provider
				value={new URLSearchParams(`id=${APP}&eventId=evt_dev`)}
			>
				{children}
			</SearchParamsContext.Provider>
		</AppRouterContext.Provider>
	);
}

/** `relist` is how the events list reads back a saved event; by default it never does. */
async function mountEditor(
	stored: IEvent,
	relist?: (saved: IEvent) => IEvent,
	sinkStatus: IEventSinkStatus = { active: false },
) {
	let listed = stored;
	const upserts: Array<{ event: IEvent; pat?: string; source?: string }> = [];
	const repairs: Array<{ appId: string; eventId: string }> = [];
	const pushes: string[] = [];
	const fake = await createFakeWorkspace();
	const view = await mountDevices(
		() => (
			<RouterFrame pushes={pushes}>
				<EventsPage
					eventMapping={MAPPING}
					appId={APP}
					eventId="evt_dev"
					basePath="/library/config/events"
				/>
			</RouterFrame>
		),
		{
			fake,
			providers: false,
			backend: {
				routeState: { getRoutes: async () => [] },
				pageState: { getPages: async () => [] },
				userState: {
					getProfile: async () => fake.profile,
					getInfo: async () => ({ id: fake.hub.me, dev_mode: false }),
					updateUser: async () => undefined,
					getPATs: async () => [],
					lookupUsers: async (ids: string[]) =>
						ids.map((id) => ({ id, name: `Person ${id}` })),
				},
				boardState: {
					getBoardSummaries: async () => [],
					getBoardVersions: async () => [],
					getBoard: async () => ({
						id: "board_0",
						nodes: {
							node_cron: { id: "node_cron", name: "events_cron", pins: {} },
						},
						variables: {},
					}),
					listRuns: async () => [],
				},
				eventState: {
					getEvents: async () => [listed],
					getEventAuthoritative: async () => stored,
					isEventSinkActive: async () => sinkStatus.active,
					getEventSinkStatus: async () => sinkStatus,
					repairLocalTrigger: async (appId: string, event: IEvent) => {
						repairs.push({ appId, eventId: event.id });
						return { active: true };
					},
					upsertEvent: async (
						_app: string,
						event: IEvent,
						_v: unknown,
						pat?: string,
						_tokens?: unknown,
						options?: { source?: string },
					) => {
						upserts.push({ event, pat, source: options?.source });
						const saved = {
							...event,
							updated_at: {
								secs_since_epoch: upserts.length,
								nanos_since_epoch: 0,
							},
						};
						if (relist) listed = relist(saved);
						return saved;
					},
				},
			} as never,
		},
	);
	await view.settle();
	await view.settle();
	return { ...view, upserts, repairs, pushes };
}

const toasts = () =>
	toast
		.getHistory()
		.flatMap((entry) => ("title" in entry ? [entry.title] : []));

describe("the editor of a device-only event", () => {
	test("reads Devices only, explains it and keeps the ordinary status for others", async () => {
		const device = await mountEditor(withDeviceEventSource(baseEvent));
		expect(device.container.textContent).toContain("Devices only");
		expect(device.container.textContent).toContain(
			"Runs only on the devices you deploy it to. The hub and this computer don't start it.",
		);
		expect(device.container.textContent).not.toContain("Active");
		await cleanupDevices();
		await dom.cleanup();

		const ordinary = await mountEditor(baseEvent);
		expect(ordinary.container.textContent).toContain("Active");
		expect(ordinary.container.textContent).not.toContain("Devices only");
	});

	test("Open Runs on leaves for the list with this event's devices open", async () => {
		const view = await mountEditor(withDeviceEventSource(baseEvent));
		await click(byRole("button", "Open Runs on"));
		expect(view.pushes).toEqual([
			`/library/config/events?id=${APP}&event=evt_dev`,
		]);
	});

	test("pausing it saves without a token, then names what keeps running", async () => {
		const before = toasts().length;
		const view = await mountEditor(withDeviceEventSource(baseEvent));
		await click(byRole("button", "Deactivate"));
		expect(toasts().slice(before)).toEqual([]);
		await click(byRole("button", /^Save/));
		await view.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].event.active).toBe(false);
		expect(view.upserts[0].pat).toBeUndefined();
		expect(toasts().slice(before)).toEqual([
			'"Nightly digest" saved',
			"Devices that already run it keep running until you update or stop their service.",
		]);
	});

	test("a pause that is toggled back and never saved says nothing about devices", async () => {
		const before = toasts().length;
		const view = await mountEditor(withDeviceEventSource(baseEvent));
		await click(byRole("button", "Deactivate"));
		await click(byRole("button", "Activate"));
		expect(toasts().slice(before)).toEqual([]);
		expect(view.upserts).toEqual([]);
	});

	test("saving clears Unsaved changes although the list reads the record back reshaped", async () => {
		const view = await mountEditor(
			withDeviceEventSource(baseEvent),
			(saved) => ({
				...saved,
				inputs: [],
			}),
		);
		await click(byRole("button", "Deactivate"));
		expect(view.container.textContent).toContain("Unsaved changes");
		await click(byRole("button", /^Save/));
		await view.settle();
		expect(view.container.textContent).not.toContain("Unsaved changes");
		expect(view.container.textContent).toContain("All changes saved");
	});

	test("a list response older than the save neither reverts the draft nor marks it unsaved", async () => {
		const stored = withDeviceEventSource(baseEvent);
		const view = await mountEditor(stored, () => ({ ...stored }));
		await click(byRole("button", "Deactivate"));
		await click(byRole("button", /^Save/));
		await view.settle();
		expect(byRole("button", "Activate")).toBeTruthy();
		expect(view.container.textContent).toContain("All changes saved");
	});

	test("activating a trigger on this computer saves through the host, which asks for any token", async () => {
		const view = await mountEditor({ ...baseEvent, active: false });
		await click(byRole("button", "Activate"));
		await click(byRole("button", /^Save/));
		await view.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].event.active).toBe(true);
		expect(view.upserts[0].pat).toBeUndefined();
	});

	test("pausing a trigger on this computer saves without a token", async () => {
		const view = await mountEditor(baseEvent);
		await click(byRole("button", "Deactivate"));
		await click(byRole("button", /^Save/));
		await view.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].pat).toBeUndefined();
	});

	test("activating a hub trigger saves without a token, as it runs as the user who saved it", async () => {
		const view = await mountEditor({
			...baseEvent,
			active: false,
			execution_mode: "Remote",
		} as IEvent);
		await click(byRole("button", "Activate"));
		await click(byRole("button", /^Save/));
		await view.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].pat).toBeUndefined();
	});
});

describe("the editor of a hub trigger and the user it runs as", () => {
	const hubEvent = { ...baseEvent, execution_mode: "Remote" } as IEvent;
	const PAUSED = "Paused: the person it ran as can no longer run it";

	test("says it is paused and turns it on as you from the warning", async () => {
		const view = await mountEditor(hubEvent, undefined, {
			active: false,
			run_as_user_id: null,
		});
		expect(view.container.textContent).toContain(PAUSED);
		expect(view.container.textContent).not.toContain("left");
		await click(byRole("button", "Turn on"));
		await view.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].event.active).toBe(true);
		expect(view.upserts[0].pat).toBeUndefined();
		expect(toasts()).toContain("Nightly digest now runs as you.");
	});

	test("a trigger without a user only notes that saving runs it as you", async () => {
		const view = await mountEditor(hubEvent, undefined, {
			active: true,
			run_as_user_id: null,
		});
		expect(view.container.textContent).toContain("Runs without a user");
		expect(view.container.textContent).toContain(
			"Saving it or turning it on again runs it as you.",
		);
		expect(view.container.textContent).not.toContain(PAUSED);
	});

	test("a legacy trigger says it runs with its saved token until someone saves or turns it on", async () => {
		const view = await mountEditor(hubEvent, undefined, {
			active: true,
			run_as_user_id: null,
			legacy_token: true,
		});
		expect(view.container.textContent).toContain(
			"Runs with a saved token until someone saves or turns it on",
		);
		expect(view.container.textContent).not.toContain("Runs without a user");
		expect(view.container.textContent).not.toContain(PAUSED);
	});

	test("a trigger with its user names that user by display name", async () => {
		const view = await mountEditor(hubEvent, undefined, {
			active: true,
			run_as_user_id: "user_ada",
		});
		await advance(30);
		await view.settle();
		expect(
			view.container.querySelector("[data-run-as='bound']")?.textContent,
		).toBe("Runs as Person user_ada");
	});

	test("a Local trigger whose sink is on the hub names whom it runs as too", async () => {
		const view = await mountEditor(baseEvent, undefined, {
			active: true,
			run_as_user_id: "user_ada",
		});
		await advance(30);
		await view.settle();
		expect(
			view.container.querySelector("[data-run-as='bound']")?.textContent,
		).toBe("Runs as Person user_ada");
	});
});

describe("the editor of a trigger on this computer without a token", () => {
	test("says it doesn't run here, and Add token repairs only this computer", async () => {
		const before = toasts().length;
		const view = await mountEditor(baseEvent, undefined, {
			active: false,
			needs_token: true,
		});
		expect(view.container.textContent).toContain(
			"Doesn't run on this computer",
		);
		expect(view.container.textContent).toContain(
			"Its trigger runs on this computer but has no personal access token to sign in to the hub, so it never fires.",
		);
		await click(byRole("button", "Add token"));
		await view.settle();
		expect(view.repairs).toEqual([{ appId: APP, eventId: "evt_dev" }]);
		expect(view.upserts).toEqual([]);
		expect(queryByRole("dialog")).toBeNull();
		expect(toasts().slice(before)).toEqual([
			"Nightly digest now runs on this computer.",
		]);
	});

	test("a dismissed token prompt stays silent; any other failure says so", async () => {
		const dismissed = Object.assign(new Error("cancelled"), {
			isLocalSinkConsentDismissed: true,
		});
		const failures = [dismissed, new Error("hub unavailable")];
		const before = toasts().length;
		const view = await mountEditor(baseEvent, undefined, {
			active: false,
			needs_token: true,
		});
		const { useBackendStore } = await import("../../../state/backend-state");
		const eventState = useBackendStore.getState().backend?.eventState as {
			repairLocalTrigger?: unknown;
		};
		eventState.repairLocalTrigger = async () => {
			throw failures.shift();
		};
		await click(byRole("button", "Add token"));
		await view.settle();
		expect(toasts().slice(before)).toEqual([]);
		await click(byRole("button", "Add token"));
		await view.settle();
		expect(toasts().slice(before)).toEqual([
			"Could not add a token to Nightly digest.",
		]);
	});
});
