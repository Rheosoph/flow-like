import { afterAll, afterEach, expect, test } from "bun:test";
import { act } from "react";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IEventState } from "../../../state/backend-state/event-state";
import {
	byRole,
	click,
	installDom,
	typeInto,
} from "../devices/testing/dom-harness";

const dom = installDom();
const { mountDevices, cleanupDevices, preloadDevices, eventRecord } =
	await import("../devices/testing/mount-devices");
await preloadDevices();
const { NewEventDeployment } = await import("./new-event-deployment");
const { APPS } = await import(
	"../../../lib/device-management/model/__fixtures__/apps"
);
const { SAMPLE_IDS } = await import(
	"../../../lib/device-management/model/__fixtures__/sample-fleet"
);

const APP = "app_visitor_checkin";
const draft: Partial<IEvent> = {
	name: "Visitor welcome",
	board_id: "flow_welcome",
	node_id: "start",
	event_type: "quick_action",
	board_version: [1, 0, 0],
	config: [],
};

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	globalThis.sessionStorage.clear();
});
afterAll(dom.restore);

function checkbox(name: string): HTMLInputElement {
	const label = Array.from(document.querySelectorAll("label")).find((row) =>
		row.textContent?.includes(name),
	);
	const input = label?.querySelector<HTMLInputElement>("input");
	if (!input) throw new Error(`No device checkbox for ${name}`);
	return input;
}

test("an older hub cannot create a device event", async () => {
	let creates = 0;
	const view = await mountDevices(
		({ fake, overrides }) => {
			const readPlacements = fake.hub.appPlacements.bind(fake.hub);
			fake.hub.appPlacements = (appId) => {
				const { device_event_creation: _supported, ...placements } =
					readPlacements(appId);
				return placements;
			};
			return (
				<NewEventDeployment
					appId={APP}
					draftEvent={draft}
					overrides={overrides}
					onCreate={async () => {
						creates += 1;
						throw new Error("Unexpected creation");
					}}
				/>
			);
		},
		{ providers: false },
	);
	await click(checkbox("edge-berlin-01"));
	expect(view.container.textContent).toContain(
		"Update your hub to create events directly on devices.",
	);
	expect(
		(byRole("button", "Create & deploy") as HTMLButtonElement).disabled,
	).toBe(true);
	expect((byRole("button", "Create only") as HTMLButtonElement).disabled).toBe(
		true,
	);
	expect(creates).toBe(0);
});

test("Create only saves without selecting or deploying a device and uses the footer", async () => {
	let creates = 0;
	const completed: IEvent[] = [];
	const saved = eventRecord(APPS[APP].events[0]) as unknown as IEvent;
	const footer = document.createElement("footer");
	document.body.append(footer);
	const view = await mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={draft}
				overrides={overrides}
				footerContainer={footer}
				onCreate={async () => {
					creates += 1;
					return saved;
				}}
				onComplete={(event) => completed.push(event)}
			/>
		),
		{ providers: false },
	);
	expect(footer.textContent).toContain("Create & deploy");
	expect(view.container.textContent).not.toContain("Create only");
	expect((byRole("button", "Create only") as HTMLButtonElement).disabled).toBe(
		false,
	);
	expect(
		(byRole("button", "Create & deploy") as HTMLButtonElement).disabled,
	).toBe(true);
	await click(byRole("button", "Create only"));
	await view.settle();
	expect(creates).toBe(1);
	expect(completed).toEqual([saved]);
	expect(view.fake.api.commands.filter(([, type]) => type === "apply")).toEqual(
		[],
	);
	footer.remove();
});

test("offline event creation does not require the hub capability", async () => {
	let creates = 0;
	const saved = eventRecord(APPS.app_crm_sync.events[0]) as unknown as IEvent;
	const view = await mountDevices(
		({ fake, overrides }) => {
			const readPlacements = fake.hub.appPlacements.bind(fake.hub);
			fake.hub.appPlacements = (appId) => {
				const { device_event_creation: _supported, ...placements } =
					readPlacements(appId);
				return placements;
			};
			return (
				<NewEventDeployment
					appId="app_crm_sync"
					draftEvent={draft}
					overrides={overrides}
					onCreate={async () => {
						creates += 1;
						return saved;
					}}
				/>
			);
		},
		{ providers: false },
	);
	await click(byRole("button", "Create only"));
	await view.settle();
	expect(creates).toBe(1);
	expect(view.fake.api.commands.filter(([, type]) => type === "apply")).toEqual(
		[],
	);
});

test("selected device chips remove choices and filters report device facts", async () => {
	const view = await mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={draft}
				overrides={overrides}
				onCreate={async () => {
					throw new Error("Unexpected creation");
				}}
			/>
		),
		{ providers: false },
	);
	await click(checkbox("edge-berlin-01"));
	await click(byRole("button", /^Ready/));
	expect(view.container.textContent).toContain("edge-berlin-01");
	await click(byRole("button", /^Runs this app/));
	expect(byRole("button", /^Runs this app/).getAttribute("aria-pressed")).toBe(
		"true",
	);
	await click(byRole("button", "Remove edge-berlin-01"));
	expect(
		(byRole("button", "Create & deploy") as HTMLButtonElement).disabled,
	).toBe(true);
	await click(byRole("button", /^All/));
	expect(checkbox("edge-berlin-01").checked).toBe(false);
});

test("a schedule replaces its target when another device is selected", async () => {
	const schedule = {
		...draft,
		event_type: "cron",
		config: [
			...new TextEncoder().encode(
				JSON.stringify({
					expression: "0 * * * * *",
					timezone: "Europe/Berlin",
				}),
			),
		],
	};
	const view = await mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={schedule}
				overrides={overrides}
				onCreate={async () => {
					throw new Error("Unexpected creation");
				}}
			/>
		),
		{ providers: false },
	);
	await click(checkbox("edge-berlin-01"));
	await click(checkbox("studio-mac-mini"));
	expect(checkbox("edge-berlin-01").checked).toBe(false);
	expect(checkbox("studio-mac-mini").checked).toBe(true);
	expect(view.container.textContent).toContain("1 selected");
	expect(view.container.textContent).toContain(
		"Schedules and bots run on one device at a time.",
	);
});

test("a selected device becoming unavailable blocks creation", async () => {
	let creates = 0;
	const view = await mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={draft}
				overrides={overrides}
				onCreate={async () => {
					creates += 1;
					throw new Error("Unexpected creation");
				}}
			/>
		),
		{ providers: false },
	);
	await click(checkbox("edge-berlin-01"));
	expect(
		(byRole("button", "Create & deploy") as HTMLButtonElement).disabled,
	).toBe(false);
	await act(async () => {
		view.fake.hub.rows.delete(SAMPLE_IDS.edge);
		await view.fake.queryClient.invalidateQueries({ queryKey: ["devices"] });
	});
	await view.settle();
	expect(
		(byRole("button", "Create & deploy") as HTMLButtonElement).disabled,
	).toBe(true);
	expect(view.container.textContent).toContain(
		"A selected device is no longer available.",
	);
	expect(creates).toBe(0);
});

test("a failed refresh retries the same saved definition without creating again", async () => {
	let creates = 0;
	let failRead = false;
	const saved = {
		...eventRecord(APPS[APP].events[0]),
		id: "new-device-event",
	} as unknown as IEvent;
	const getEvents = async () => {
		if (failRead) throw new Error("Event list unavailable");
		return [...APPS[APP].events.map(eventRecord), ...(creates ? [saved] : [])];
	};
	const view = await mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={draft}
				overrides={overrides}
				onCreate={async () => {
					creates += 1;
					failRead = true;
					return saved;
				}}
			/>
		),
		{
			providers: false,
			backend: { eventState: { getEvents } as unknown as IEventState },
		},
	);
	await click(checkbox("edge-berlin-01"));
	await click(byRole("button", "Create & deploy"));
	await view.settle();
	expect(creates).toBe(1);
	expect(
		view.container.querySelector('[data-new-event-deployment="saved"]'),
	).not.toBeNull();
	failRead = false;
	await click(byRole("button", "Retry"));
	await view.settle();
	expect(creates).toBe(1);
	expect(byRole("tab", "Settings")).toBeTruthy();
});

test("the dialog searches real device choices and waits for a target before creating", async () => {
	let creates = 0;
	const view = await mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={draft}
				overrides={overrides}
				onCreate={async () => {
					creates += 1;
					throw new Error("Save failed");
				}}
			/>
		),
		{ providers: false },
	);
	const create = byRole("button", "Create & deploy") as HTMLButtonElement;
	expect(create.disabled).toBe(true);
	expect(creates).toBe(0);
	await typeInto(byRole("textbox", "Search devices"), "edge-berlin");
	expect(view.container.textContent).toContain("edge-berlin-01");
	expect(view.container.textContent).not.toContain("studio-mac-mini");
	await click(checkbox("edge-berlin-01"));
	await view.settle();
	expect(create.disabled).toBe(false);
	await click(create);
	await view.settle();
	expect(creates).toBe(1);
	expect(view.container.textContent).toContain("Save failed");
	expect(checkbox("edge-berlin-01").checked).toBe(true);
	expect(view.fake.api.commands.filter(([, type]) => type === "apply")).toEqual(
		[],
	);
});

test("a server-only trigger cannot create a device event", async () => {
	let creates = 0;
	const view = await mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={{ ...draft, event_type: "teams" }}
				overrides={overrides}
				onCreate={async () => {
					creates += 1;
					throw new Error("Unexpected creation");
				}}
			/>
		),
		{ providers: false },
	);
	await click(checkbox("edge-berlin-01"));
	expect(view.container.textContent).toContain("Teams bots run on the hub.");
	expect(
		(byRole("button", "Create & deploy") as HTMLButtonElement).disabled,
	).toBe(true);
	expect(creates).toBe(0);
});

test("creation and restarting a failed deployment retain the saved event inside the dialog", async () => {
	let creates = 0;
	const saved = eventRecord(APPS[APP].events[0]) as unknown as IEvent;
	const busy: boolean[] = [];
	const persisted: boolean[] = [];
	const view = await mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={draft}
				overrides={overrides}
				onBusyChange={(value) => busy.push(value)}
				onSavedChange={(value) => persisted.push(value)}
				onCreate={async () => {
					creates += 1;
					return saved;
				}}
			/>
		),
		{ providers: false },
	);
	await click(checkbox("edge-berlin-01"));
	await click(byRole("button", "Create & deploy"));
	await view.settle();
	expect(creates).toBe(1);
	expect(
		view.container.querySelector('[data-new-event-deployment="saved"]'),
	).not.toBeNull();
	expect(view.container.textContent).toContain(
		"Event saved for device deployment.",
	);
	expect(byRole("tab", "Settings")).toBeTruthy();
	expect(byRole("tab", "Review")).toBeTruthy();
	expect(busy.at(-1)).toBe(false);
	expect(persisted.at(-1)).toBe(true);
	await click(byRole("tab", "Review"));
	await view.settle();
	expect(view.navigations).toEqual([]);
	expect(view.fake.api.commands.filter(([, type]) => type === "apply")).toEqual(
		[],
	);
	await click(byRole("tab", "Access & cost"));
	await view.settle();
	await click(byRole("checkbox", /I own Visitor Check-in/));
	await click(byRole("tab", "Review"));
	await view.settle();
	await click(byRole("button", "Deploy Check-in page to edge-berlin-01"));
	await view.settle();
	expect(busy.at(-1)).toBe(false);
	await click(byRole("button", "Start over…"));
	await view.settle();
	expect(byRole("tab", "Devices").getAttribute("aria-selected")).toBe("true");
	expect(
		view.container.querySelector('[data-new-event-deployment="saved"]'),
	).not.toBeNull();
	expect(persisted.at(-1)).toBe(true);
	expect(creates).toBe(1);
	expect(view.navigations).toEqual([]);
});
