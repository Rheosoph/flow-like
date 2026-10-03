import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { type ReactNode, useMemo } from "react";
import { withDeviceEventSource } from "../../../lib/event-source";
import type { IEvent } from "../../../lib/schema/flow/event";
import {
	byRole,
	click,
	inPortal,
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

async function mountEditor(stored: IEvent) {
	const upserts: Array<{ event: IEvent; pat?: string; source?: string }> = [];
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
					getEvents: async () => [stored],
					getEventAuthoritative: async () => stored,
					isEventSinkActive: async () => false,
					upsertEvent: async (
						_app: string,
						event: IEvent,
						_v: unknown,
						pat?: string,
						_tokens?: unknown,
						options?: { source?: string },
					) => {
						upserts.push({ event, pat, source: options?.source });
						return event;
					},
				},
			} as never,
		},
	);
	await view.settle();
	await view.settle();
	return { ...view, upserts, pushes };
}

const toasts = () => toast.getHistory().map((entry) => entry.title);

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

	test("pausing it names what keeps running and saving asks for no token", async () => {
		const before = toasts().length;
		const view = await mountEditor(withDeviceEventSource(baseEvent));
		await click(byRole("button", "Deactivate"));
		expect(toasts().slice(before)).toEqual([
			"Devices that already run it keep running until you update or stop their service.",
		]);
		await click(byRole("button", /^Save/));
		await view.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].event.active).toBe(false);
		expect(view.upserts[0].pat).toBeUndefined();
	});

	test("an ordinary sink event asks for a token when saved", async () => {
		const view = await mountEditor(baseEvent);
		await click(byRole("button", "Deactivate"));
		await click(byRole("button", /^Save/));
		await view.settle();
		expect(inPortal("dialog").textContent).toContain("Personal Access Token");
		expect(view.upserts).toEqual([]);
	});
});
