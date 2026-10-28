import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { type ReactNode, useMemo } from "react";
import { withDeviceEventSource } from "../../../lib/event-source";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IEventSinkStatus } from "../../../state/backend-state/event-state";
import {
	advance,
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

async function mountList(
	events: IEvent[] = EVENTS,
	sinkStatus: (eventId: string) => IEventSinkStatus = () => ({ active: false }),
) {
	const upserts: Array<{ event: IEvent; pat?: string }> = [];
	const repairs: Array<{ appId: string; eventId: string }> = [];
	const deleted: string[] = [];
	const nodes = Object.fromEntries(
		events.map((event) => [event.node_id, { name: "events_cron" }]),
	);
	const fake = await createFakeWorkspace();
	const view = await mountDevices(
		({ overrides }) => (
			<RouterFrame>
				<EventsOverview
					events={events}
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
					lookupUsers: async (ids: string[]) =>
						ids.map((id) => ({ id, name: `Person ${id}` })),
				},
				boardState: {
					getBoardSummaries: async () => [],
					getBoard: async () => ({ nodes }),
					listRuns: async () => [],
				},
				eventState: {
					getEvents: async () => events,
					getEventAuthoritative: async (_app: string, eventId: string) =>
						events.find((event) => event.id === eventId),
					isEventSinkActive: async (eventId: string) =>
						sinkStatus(eventId).active,
					getEventSinkStatus: async (eventId: string) => sinkStatus(eventId),
					repairLocalTrigger: async (appId: string, event: IEvent) => {
						repairs.push({ appId, eventId: event.id });
						return { active: true };
					},
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
	return { ...view, row, upserts, repairs, deleted };
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

	test("pausing an ordinary sink event asks for no token either", async () => {
		const view = await mountList();
		await click(byRole("button", "Pause event", view.row("evt_hub")));
		await view.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].event.active).toBe(false);
		expect(view.upserts[0].pat).toBeUndefined();
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

const toastTitles = (from: number) =>
	toast
		.getHistory()
		.slice(from)
		.flatMap((entry) => ("title" in entry ? [entry.title] : []));

describe("whom a hub trigger runs as in the Events list", () => {
	const remote = (id: string, name: string) =>
		({ ...cron(id, name, false), execution_mode: "Remote" }) as IEvent;
	const STATUSES: Record<string, IEventSinkStatus> = {
		evt_paused: { active: false, run_as_user_id: null },
		evt_unbound: { active: true, run_as_user_id: null },
		evt_legacy: { active: true, run_as_user_id: null, legacy_token: true },
		evt_bound: { active: true, run_as_user_id: "user_ada" },
	};
	const mountRunAs = async () => {
		const view = await mountList(
			[
				remote("evt_paused", "Weekly report"),
				remote("evt_unbound", "Daily sync"),
				remote("evt_legacy", "Monthly close"),
				remote("evt_bound", "Hourly import"),
			],
			(eventId) => STATUSES[eventId] ?? { active: false },
		);
		await advance(30);
		await view.settle();
		return view;
	};
	const runAs = (row: HTMLElement) =>
		row.querySelector<HTMLElement>("[data-run-as]");

	test("names the user, a trigger without one, a legacy token and the paused one", async () => {
		const view = await mountRunAs();
		expect(runAs(view.row("evt_bound"))?.textContent).toBe(
			"Runs as Person user_ada",
		);
		expect(runAs(view.row("evt_unbound"))?.textContent).toBe(
			"Runs without a user",
		);
		expect(runAs(view.row("evt_legacy"))?.textContent).toBe(
			"Runs with a saved token until someone saves or turns it on",
		);
		expect(runAs(view.row("evt_paused"))?.textContent).toBe(
			"Paused: the person it ran as can no longer run it",
		);
		expect(view.container.textContent).not.toContain("left");
		expect(
			view.row("evt_paused").querySelector("[data-sink-chip='off']"),
		).toBeNull();
	});

	test("warns about the paused one and turns it on as you without a token", async () => {
		const before = toast.getHistory().length;
		const view = await mountRunAs();
		expect(view.container.textContent).toContain("1 event paused");
		expect(view.container.textContent).toContain(
			"The person each one ran as can no longer run it. Turning one on runs it as you.",
		);
		await click(byRole("button", "Turn on Weekly report"));
		await view.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].event.id).toBe("evt_paused");
		expect(view.upserts[0].event.active).toBe(true);
		expect(view.upserts[0].pat).toBeUndefined();
		expect(toastTitles(before)).toEqual(["Weekly report now runs as you."]);
	});

	test("the paused row's toggle reads Turn on and turns it on as you, never pauses it", async () => {
		const before = toast.getHistory().length;
		const view = await mountRunAs();
		const paused = view.row("evt_paused");
		expect(queryByRole("button", "Pause event", paused)).toBeNull();
		await click(byRole("button", "Turn on", paused));
		await view.settle();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].event.id).toBe("evt_paused");
		expect(view.upserts[0].event.active).toBe(true);
		expect(toastTitles(before)).toEqual(["Weekly report now runs as you."]);
		expect(byRole("button", "Pause event", view.row("evt_bound"))).toBeTruthy();
	});

	test("a Local trigger whose sink is on the hub names whom it runs as", async () => {
		const view = await mountList(
			[cron("evt_local_hub", "Nightly export", false)],
			() => ({ active: true, run_as_user_id: "user_ada" }),
		);
		await advance(30);
		await view.settle();
		expect(runAs(view.row("evt_local_hub"))?.textContent).toBe(
			"Runs as Person user_ada",
		);
	});

	test("a hosted page names whose access its visitors use", async () => {
		const hosted = {
			...cron("evt_hosted", "Support chat", false),
			event_type: "simple_chat",
			execution_mode: "Remote",
			config: [
				...new TextEncoder().encode(
					JSON.stringify({ frontend_hosting: { enabled: true } }),
				),
			],
		} as IEvent;
		const view = await mountList([hosted], () => ({
			active: true,
			run_as_user_id: "user_ada",
		}));
		await advance(30);
		await view.settle();
		expect(runAs(view.row("evt_hosted"))?.textContent).toBe(
			"Visitors use Person user_ada's access to run events",
		);
	});

	test("an Internal page or an ontology action has no visitors, even with hosting switched on", async () => {
		const hosting = [
			...new TextEncoder().encode(
				JSON.stringify({
					expression: "0 * * * *",
					frontend_hosting: { enabled: true },
				}),
			),
		];
		const internalPage = {
			...cron("evt_internal", "Team intake", false),
			default_page_id: "page_intake",
			execution_mode: "Remote",
			exposure: "INTERNAL",
			config: hosting,
		} as IEvent;
		const ontologyAction = {
			...cron("evt_action", "Approve order", false),
			event_type: "ontology_action",
			default_page_id: "page_approve",
			execution_mode: "Remote",
			config: hosting,
		} as IEvent;
		const asked: string[] = [];
		const view = await mountList([internalPage, ontologyAction], (eventId) => {
			asked.push(eventId);
			return { active: true, run_as_user_id: "user_ada" };
		});
		await advance(30);
		await view.settle();
		expect(runAs(view.row("evt_internal"))?.textContent).toBe(
			"Runs as Person user_ada",
		);
		expect(runAs(view.row("evt_action"))).toBeNull();
		expect(asked).not.toContain("evt_action");
	});
});

describe("a trigger on this computer without a token in the Events list", () => {
	const STATUS: IEventSinkStatus = { active: false, needs_token: true };

	test("reads Needs a token instead of Not running, and Add token repairs only this computer", async () => {
		const before = toast.getHistory().length;
		const view = await mountList(
			[cron("evt_local", "Hourly sync", false)],
			() => STATUS,
		);
		const row = view.row("evt_local");
		expect(
			row.querySelector("[data-sink-chip='needs-token']")?.textContent,
		).toBe("Needs a token");
		expect(row.querySelector("[data-sink-chip='off']")).toBeNull();
		await click(byRole("button", "Add token", row));
		await view.settle();
		expect(view.repairs).toEqual([{ appId: APP, eventId: "evt_local" }]);
		expect(view.upserts).toEqual([]);
		expect(queryByRole("dialog")).toBeNull();
		expect(toastTitles(before)).toEqual([
			"Hourly sync now runs on this computer.",
		]);
	});

	test("a trigger that failed to start shows why on Not running", async () => {
		const view = await mountList(
			[cron("evt_local", "Hourly sync", false)],
			() => ({
				active: false,
				failed_reason: "The port is already in use",
			}),
		);
		const off = view
			.row("evt_local")
			.querySelector<HTMLElement>("[data-sink-chip='off']");
		expect(off?.textContent).toBe("Not running");
		expect(off?.title).toBe("The port is already in use");
		expect(
			queryByRole("button", "Add token", view.row("evt_local")),
		).toBeNull();
	});
});
