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
const { AppRouterContext } = await import(
	"next/dist/shared/lib/app-router-context.shared-runtime"
);
const { SearchParamsContext } = await import(
	"next/dist/shared/lib/hooks-client-context.shared-runtime"
);
const { createFakeWorkspace } = await import(
	"../devices/testing/fake-workspace"
);
const { EventsOverview } = await import("./events-overview");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const APP = "app_invoice_ai";
const HUB_ON = { standalone: { enabled: true } } as never;
const at = { secs_since_epoch: 0, nanos_since_epoch: 0 };

const cron = (id: string, name: string, device: boolean): IEvent => {
	const event = {
		id,
		name,
		description: "",
		active: true,
		board_id: "board_0",
		node_id: `node_${id}`,
		event_type: "cron",
		event_version: [1, 0, 0],
		board_version: [1, 0, 0],
		priority: 0,
		config: [
			...new TextEncoder().encode(JSON.stringify({ expression: "0 * * * *" })),
		],
		created_at: at,
		updated_at: at,
		variables: {},
	} as unknown as IEvent;
	return device ? withDeviceEventSource(event) : event;
};

const EVENTS = [
	cron("evt_dev", "Nightly digest", true),
	cron("evt_hub", "Hourly sync", false),
];
const MAPPING = {
	events_cron: { withSink: ["cron"], eventTypes: ["cron"] },
} as never;

function RouterFrame({ children }: Readonly<{ children: ReactNode }>) {
	const router = useMemo(
		() => ({
			push: () => undefined,
			replace: () => undefined,
			back: () => undefined,
			forward: () => undefined,
			refresh: () => undefined,
			prefetch: () => undefined,
		}),
		[],
	);
	return (
		<AppRouterContext.Provider value={router as never}>
			<SearchParamsContext.Provider value={new URLSearchParams(`id=${APP}`)}>
				{children}
			</SearchParamsContext.Provider>
		</AppRouterContext.Provider>
	);
}

async function mountList() {
	const upserts: Array<{ event: IEvent; pat?: string }> = [];
	const deleted: string[] = [];
	const nodes = Object.fromEntries(
		EVENTS.map((event) => [event.node_id, { name: "events_cron" }]),
	);
	const fake = await createFakeWorkspace();
	const view = await mountDevices(
		({ overrides }) => (
			<RouterFrame>
				<EventsOverview
					events={EVENTS}
					boardsMap={new Map([["board_0", "Digest flow"]])}
					appId={APP}
					eventMapping={MAPPING}
					onEdit={() => undefined}
					onDelete={(eventId) => deleted.push(eventId)}
					onNavigateToNode={() => undefined}
					onCreateEvent={() => undefined}
					hub={HUB_ON}
					devicesHarness={{ overrides }}
				/>
			</RouterFrame>
		),
		{
			fake,
			providers: false,
			backend: {
				routeState: { getRoutes: async () => [] },
				userState: {
					getProfile: async () => fake.profile,
					getInfo: async () => ({ id: fake.hub.me, dev_mode: false }),
					updateUser: async () => undefined,
					getPATs: async () => [],
				},
				boardState: {
					getBoardSummaries: async () => [],
					getBoard: async () => ({ nodes }),
					listRuns: async () => [],
				},
				eventState: {
					getEvents: async () => EVENTS,
					getEventAuthoritative: async (_app: string, eventId: string) =>
						EVENTS.find((event) => event.id === eventId),
					isEventSinkActive: async () => false,
					upsertEvent: async (
						_app: string,
						event: IEvent,
						_v: unknown,
						pat?: string,
					) => {
						upserts.push({ event, pat });
						return event;
					},
				},
			} as never,
		},
	);
	await view.settle();
	await view.settle();
	const row = (eventId: string) => {
		const found = document.getElementById(`event-row-${eventId}`);
		if (!found) throw new Error(`no row for ${eventId}`);
		return found;
	};
	return { ...view, row, upserts, deleted };
}

const chips = (row: HTMLElement) => ({
	deviceOnly: row.querySelector("[data-device-only]")?.textContent ?? null,
	notRunning: row.querySelector("[data-sink-chip='off']") !== null,
});

describe("a device-only event in the Events list", () => {
	test("reads Devices only, never Not running", async () => {
		const view = await mountList();
		expect(chips(view.row("evt_dev"))).toEqual({
			deviceOnly: "Devices only",
			notRunning: false,
		});
		expect(chips(view.row("evt_hub"))).toEqual({
			deviceOnly: null,
			notRunning: true,
		});
	});

	test("is counted under Devices only, not under Needs setup or Live", async () => {
		await mountList();
		const label = (name: string) =>
			byRole("button", new RegExp(`^${name}`)).textContent;
		expect(label("Devices only")).toBe("Devices only1");
		expect(label("Needs setup")).toBe("Needs setup1");
		expect(label("Live")).toBe("Live0");
	});

	test("pausing it asks for no token and names what keeps running", async () => {
		const before = toast.getHistory().length;
		const view = await mountList();
		await click(byRole("button", "Pause event", view.row("evt_dev")));
		await view.settle();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].event.active).toBe(false);
		expect(view.upserts[0].pat).toBeUndefined();
		expect(queryByRole("dialog")).toBeNull();
		expect(
			toast
				.getHistory()
				.slice(before)
				.flatMap((entry) => ("title" in entry ? [entry.title] : [])),
		).toEqual([
			"Devices that already run it keep running until you update or stop their service.",
		]);
	});

	test("an ordinary sink event still asks for a token", async () => {
		const view = await mountList();
		await click(byRole("button", "Pause event", view.row("evt_hub")));
		await view.settle();
		expect(inPortal("dialog").textContent).toContain("Authorize this change");
		expect(view.upserts).toEqual([]);
	});

	test("deleting asks first and deletes only after confirmation", async () => {
		const view = await mountList();
		await click(byRole("button", "Delete event", view.row("evt_dev")));
		expect(inPortal("alertdialog").textContent).toContain(
			"Delete Nightly digest?",
		);
		expect(view.deleted).toEqual([]);
		await click(byRole("button", "Delete event", inPortal("alertdialog")));
		expect(view.deleted).toEqual(["evt_dev"]);
	});
});
