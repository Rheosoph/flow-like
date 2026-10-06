import { afterAll, afterEach, expect, test } from "bun:test";
import { act, useState } from "react";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IEventState } from "../../../state/backend-state/event-state";
import {
	allByRole,
	byRole,
	click,
	installDom,
	keyDown,
	queryByRole,
	typeInto,
} from "../devices/testing/dom-harness";
import type { DeviceWorkspaceOverrides } from "../devices/workspace/device-workspace-provider";

const dom = installDom();
const { mountDevices, cleanupDevices, preloadDevices, eventRecord } =
	await import("../devices/testing/mount-devices");
await preloadDevices();
const { NewEventDeployment, newEventSeams, useLatched } = await import(
	"./new-event-deployment"
);
const { deviceRow, keySession, sampleFleet, vault } = await import(
	"../../../lib/device-management/model/__fixtures__/sample-fleet"
);
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

const footers: HTMLElement[] = [];

function mountFooter(): HTMLElement {
	const footer = document.createElement("footer");
	document.body.append(footer);
	footers.push(footer);
	return footer;
}

afterEach(async () => {
	for (const footer of footers.splice(0)) footer.remove();
	newEventSeams.slowSaveMs = 20_000;
	newEventSeams.rowRendered = undefined;
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
		(byRole("button", /^Create & deploy/) as HTMLButtonElement).disabled,
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
	const footer = mountFooter();
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
		(byRole("button", /^Create & deploy/) as HTMLButtonElement).disabled,
	).toBe(true);
	await click(byRole("button", "Create only"));
	await view.settle();
	expect(creates).toBe(1);
	expect(completed).toEqual([saved]);
	expect(view.fake.api.commands.filter(([, type]) => type === "apply")).toEqual(
		[],
	);
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
		(byRole("button", /^Create & deploy/) as HTMLButtonElement).disabled,
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
		(byRole("button", /^Create & deploy/) as HTMLButtonElement).disabled,
	).toBe(false);
	await act(async () => {
		view.fake.hub.rows.delete(SAMPLE_IDS.edge);
		await view.fake.queryClient.invalidateQueries({ queryKey: ["devices"] });
	});
	await view.settle();
	expect(
		(byRole("button", /^Create & deploy/) as HTMLButtonElement).disabled,
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
	await click(byRole("button", /^Create & deploy/));
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
	const create = byRole("button", /^Create & deploy/) as HTMLButtonElement;
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
		(byRole("button", /^Create & deploy/) as HTMLButtonElement).disabled,
	).toBe(true);
	expect(creates).toBe(0);
});

test("creation and restarting a failed deployment retain the saved event inside the dialog", async () => {
	let creates = 0;
	const saved = eventRecord(APPS[APP].events[0]) as unknown as IEvent;
	const busy: boolean[] = [];
	const persisted: (IEvent | null)[] = [];
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
	await click(byRole("button", /^Create & deploy/));
	await view.settle();
	expect(creates).toBe(1);
	expect(
		view.container.querySelector('[data-new-event-deployment="saved"]'),
	).not.toBeNull();
	expect(view.container.textContent).toContain("Check-in page saved");
	expect(view.container.textContent).toContain(
		"Deploy it to edge-berlin-01 now, or close and do it later from its Runs on column.",
	);
	expect(byRole("tab", "Settings")).toBeTruthy();
	expect(byRole("tab", "Review")).toBeTruthy();
	expect(busy.at(-1)).toBe(false);
	expect(persisted.at(-1)).toEqual(saved);
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
	expect(persisted.at(-1)).toEqual(saved);
	expect(creates).toBe(1);
	expect(view.navigations).toEqual([]);
}, 30_000);

type Seed = ReturnType<typeof sampleFleet>;

const bulkId = (index: number) =>
	`00000000-0000-4000-8000-${String(index).padStart(12, "0")}`;

/** The golden fleet plus `ready` selectable devices and `blocked` ones that are offline without keys. */
function fleetWith(ready: number, blocked: number): Seed {
	const seed = sampleFleet();
	const rowOf = (id: string) => {
		const row = seed.devices.find((item) => item.device_id === id);
		if (!row) throw new Error(`No sample device ${id}`);
		return row;
	};
	const online = rowOf(SAMPLE_IDS.studio);
	const offline = rowOf(SAMPLE_IDS.warehouse);
	for (let index = 0; index < ready + blocked; index++) {
		const usable = index < ready;
		const id = bulkId(index);
		seed.devices.push(
			deviceRow({
				...(usable ? online : offline),
				device_id: id,
				name: `bulk-${String(index).padStart(3, "0")}`,
			}),
		);
		if (!usable) continue;
		seed.keys.push(keySession(id, "unlocked"));
		seed.local.vaults.push(vault(id));
		seed.live[id] = structuredClone(seed.live[SAMPLE_IDS.studio]);
	}
	return seed;
}

const deviceLabels = () =>
	Array.from(document.querySelectorAll("label")).filter((label) =>
		label.querySelector("input"),
	);

const createButton = () =>
	byRole("button", /^Create & deploy/) as HTMLButtonElement;

function mountPanel(
	props: Partial<React.ComponentProps<typeof NewEventDeployment>> = {},
	seed?: Seed,
) {
	return mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={draft}
				overrides={overrides}
				onCreate={async () => {
					throw new Error("Unexpected creation");
				}}
				{...props}
			/>
		),
		{ providers: false, ...(seed ? { seed } : {}) },
	);
}

test("Ready is the default working set", async () => {
	await mountPanel();
	expect(byRole("button", /^Ready/).getAttribute("aria-pressed")).toBe("true");
	expect(byRole("button", /^Ready/).textContent).toContain("2");
	expect(deviceLabels()).toHaveLength(2);
});

test("a fleet with nothing ready opens on All", async () => {
	const seed = sampleFleet();
	for (const row of seed.devices)
		if (
			row.device_id === SAMPLE_IDS.edge ||
			row.device_id === SAMPLE_IDS.studio
		)
			row.last_seen_at = 1_700_000_000;
	await mountPanel({}, seed);
	expect(byRole("button", /^All/).getAttribute("aria-pressed")).toBe("true");
});

test("the filter is a labelled group and its buttons are touch sized", async () => {
	const view = await mountPanel();
	const group = view.container.querySelector(
		'[aria-label="Filter devices"]',
	) as HTMLElement;
	expect(group).not.toBeNull();
	for (const button of Array.from(group.querySelectorAll("button")))
		expect(button.className).toContain("min-h-11");
	expect(byRole("button", /^Ready/).className).toContain("focus-visible:ring");
});

test("a device row states facts and its connection in words, and a blocked one only gives its reason until Fix is opened", async () => {
	const view = await mountPanel();
	const edge = checkbox("edge-berlin-01").closest("label") as HTMLElement;
	expect(edge.textContent).toContain("linux");
	expect(edge.textContent).toContain("agent 0.9.4");
	expect(edge.textContent).toContain("Live");
	await click(byRole("button", /^All/));
	const warehouse = checkbox("warehouse-pi").closest("label") as HTMLElement;
	expect(warehouse.textContent).toContain("Offline");
	expect(checkbox("warehouse-pi").disabled).toBe(true);
	expect(queryByRole("button", /Diagnose/)).toBeNull();
	await click(byRole("button", "Fix warehouse-pi"));
	expect(byRole("button", /Diagnose/)).toBeTruthy();
	expect(view.container.textContent).not.toContain("Unexpected");
});

test("Pick all selects every ready device shown and names the target on the button", async () => {
	const footer = mountFooter();
	await mountPanel({ footerContainer: footer });
	expect(footer.textContent).toContain("Pick a device to deploy to.");
	expect(createButton().disabled).toBe(true);
	await click(byRole("button", "Pick all 2 ready devices"));
	expect(checkbox("edge-berlin-01").checked).toBe(true);
	expect(checkbox("studio-mac-mini").checked).toBe(true);
	expect(queryByRole("button", /^Pick all/)).toBeNull();
	expect(createButton().textContent).toBe("Create & deploy to 2 devices");
	expect(footer.textContent).not.toContain("Pick a device");
	expect(createButton().disabled).toBe(false);
});

test("a single device names itself on the button and Pick all stays away from schedules", async () => {
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
	await mountPanel({ draftEvent: schedule });
	expect(queryByRole("button", /^Pick all/)).toBeNull();
	await click(checkbox("edge-berlin-01"));
	expect(createButton().textContent).toBe("Create & deploy to edge-berlin-01");
});

test("selected chips stop at three and the rest open the selected-only view", async () => {
	await mountPanel({}, fleetWith(6, 0));
	await click(byRole("button", "Pick all 8 ready devices"));
	const group = document.querySelector(
		'[aria-label="Selected devices"]',
	) as HTMLElement;
	const chips = allByRole("button", /^Remove /, group);
	expect(chips).toHaveLength(3);
	const more = byRole("button", "+5 more", group);
	await click(byRole("button", /^Ready/));
	await click(more);
	expect(deviceLabels()).toHaveLength(8);
	expect(byRole("button", /selected/).getAttribute("aria-pressed")).toBe(
		"true",
	);
});

test("a large fleet renders a capped list, shows more on request and keeps the rows that stay still while searching", async () => {
	const seed = fleetWith(0, 300);
	const renders = new Map<string, number>();
	newEventSeams.rowRendered = (id) =>
		renders.set(id, (renders.get(id) ?? 0) + 1);
	await mountPanel({}, seed);
	await click(byRole("button", /^All/));
	expect(byRole("button", /^All/).textContent).toContain("305");
	expect(deviceLabels()).toHaveLength(100);
	await click(byRole("button", "Show 100 more"));
	expect(deviceLabels()).toHaveLength(200);
	await click(byRole("button", "Show 100 more"));
	await click(byRole("button", "Show 5 more"));
	expect(deviceLabels()).toHaveLength(305);
	expect(queryByRole("button", /^Show \d+ more/)).toBeNull();
	await click(byRole("button", /^Ready/));
	await click(byRole("button", /^All/));
	expect(deviceLabels()).toHaveLength(100);
	const nameOf = (label: HTMLElement) =>
		label.querySelector("span span")?.textContent ?? "";
	const namesBefore = deviceLabels().map(nameOf);
	const idOf = new Map(seed.devices.map((row) => [row.name, row.device_id]));
	renders.clear();
	await typeInto(byRole("textbox", "Search devices"), "bulk-");
	const kept = deviceLabels()
		.map(nameOf)
		.filter((name) => namesBefore.includes(name));
	expect(kept.length).toBeGreaterThan(50);
	for (const name of kept)
		expect(renders.get(idOf.get(name) ?? "")).toBeUndefined();
}, 30_000);

test("blocked rows of a large fleet mount no fix actions until Fix is opened", async () => {
	await mountPanel({}, fleetWith(0, 120));
	await click(byRole("button", /^All/));
	expect(queryByRole("button", /^Fix /)).not.toBeNull();
	expect(queryByRole("button", /Diagnose|Restore keys/)).toBeNull();
}, 30_000);

test("a server-only or older-hub reason is also in the footer next to the disabled action", async () => {
	const footer = mountFooter();
	await mountPanel({
		footerContainer: footer,
		draftEvent: { ...draft, event_type: "teams" },
	});
	await click(checkbox("edge-berlin-01"));
	expect(footer.textContent).toContain("Teams bots run on the hub.");
	expect(createButton().disabled).toBe(true);
});

test("a selected device that disappears is explained in the footer", async () => {
	const footer = mountFooter();
	const view = await mountPanel({ footerContainer: footer });
	await click(checkbox("edge-berlin-01"));
	await act(async () => {
		view.fake.hub.rows.delete(SAMPLE_IDS.edge);
		await view.fake.queryClient.invalidateQueries({ queryKey: ["devices"] });
	});
	await view.settle();
	expect(footer.textContent).toContain(
		"A selected device is no longer available.",
	);
	expect(createButton().disabled).toBe(true);
});

test("an unsaved draft is never written to session storage; the saved event's draft is", async () => {
	const saved = {
		...eventRecord(APPS[APP].events[0]),
		id: "persisted-event",
	} as unknown as IEvent;
	const keys = (part: string) =>
		Object.keys(globalThis.sessionStorage).filter((key) => key.includes(part));
	const view = await mountPanel({ onCreate: async () => saved });
	await click(checkbox("edge-berlin-01"));
	expect(keys("new-event-")).toEqual([]);
	expect(keys("persisted-event")).toEqual([]);
	await click(createButton());
	await view.settle();
	expect(keys("new-event-")).toEqual([]);
	expect(keys("persisted-event").length).toBeGreaterThan(0);
});

test("the panel releases the dialog's busy and saved flags when it unmounts", async () => {
	const saved = eventRecord(APPS[APP].events[0]) as unknown as IEvent;
	const busy: boolean[] = [];
	const persisted: (IEvent | null)[] = [];
	let finish: (event: IEvent) => void = () => {};
	const view = await mountPanel({
		onBusyChange: (value) => busy.push(value),
		onSavedChange: (value) => persisted.push(value),
		onCreate: () =>
			new Promise<IEvent>((resolve) => {
				finish = resolve;
			}),
	});
	await click(checkbox("edge-berlin-01"));
	await click(createButton());
	expect(busy.at(-1)).toBe(true);
	await view.unmount();
	expect(busy.at(-1)).toBe(false);
	expect(persisted.at(-1)).toBeNull();
	finish(saved);
});

test("a save that runs long releases the dialog and says the id is reserved", async () => {
	newEventSeams.slowSaveMs = 0;
	const footer = mountFooter();
	const busy: boolean[] = [];
	const saved = eventRecord(APPS[APP].events[0]) as unknown as IEvent;
	let finish: (event: IEvent) => void = () => {};
	const view = await mountPanel({
		footerContainer: footer,
		onBusyChange: (value) => busy.push(value),
		onCreate: () =>
			new Promise<IEvent>((resolve) => {
				finish = resolve;
			}),
	});
	await click(checkbox("edge-berlin-01"));
	await click(createButton());
	await view.settle();
	expect(busy).toContain(true);
	expect(busy.at(-1)).toBe(false);
	expect(footer.textContent).toContain(
		"Taking longer than usual. You can close this; the event id is reserved, so nothing is lost.",
	);
	await act(async () => finish(saved));
	await view.settle();
	expect(
		view.container.querySelector('[data-new-event-deployment="saved"]'),
	).not.toBeNull();
});

test("Create only stays on the creation view while the event list refreshes", async () => {
	const completed: IEvent[] = [];
	const saved = {
		...eventRecord(APPS[APP].events[0]),
		id: "created-only",
	} as unknown as IEvent;
	let release: () => void = () => {};
	const gate = new Promise<void>((resolve) => {
		release = resolve;
	});
	let hold = false;
	const getEvents = async () => {
		if (hold) await gate;
		return APPS[APP].events.map(eventRecord);
	};
	const persisted: (IEvent | null)[] = [];
	const view = await mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={draft}
				overrides={overrides}
				onSavedChange={(value) => persisted.push(value)}
				onComplete={(event) => completed.push(event)}
				onCreate={async () => {
					hold = true;
					return saved;
				}}
			/>
		),
		{
			providers: false,
			backend: { eventState: { getEvents } as unknown as IEventState },
		},
	);
	await click(byRole("button", "Create only"));
	expect(persisted.filter(Boolean)).toEqual([]);
	expect(
		view.container.querySelector('[data-new-event-deployment="saved"]'),
	).toBeNull();
	expect(view.container.textContent).not.toContain("Deploy it to");
	expect(completed).toEqual([]);
	await act(async () => release());
	await view.settle();
	expect(completed).toEqual([saved]);
});

test("a saved event lands on the step that needs input, with a heading to focus, an announcement and linked tabs", async () => {
	const saved = eventRecord(APPS[APP].events[0]) as unknown as IEvent;
	const view = await mountPanel({ onCreate: async () => saved });
	await click(checkbox("edge-berlin-01"));
	await click(createButton());
	await view.settle();
	expect(byRole("tab", "Access & cost").getAttribute("aria-selected")).toBe(
		"true",
	);
	const heading = byRole("heading", "Check-in page saved");
	expect(document.activeElement).toBe(heading);
	expect(view.container.textContent).toContain("Event saved");
	const tab = byRole("tab", "Access & cost");
	const panel = document.getElementById(
		tab.getAttribute("aria-controls") ?? "",
	);
	expect(panel?.getAttribute("role")).toBe("tabpanel");
	expect(panel?.getAttribute("aria-labelledby")).toBe(tab.id);
	expect(tab.className).toContain("min-h-11");
	await keyDown(tab, "ArrowRight");
	await view.settle();
	expect(byRole("tab", "Review").getAttribute("aria-selected")).toBe("true");
}, 30_000);

test("the footer carries Review, a blocked Deploy with its reason and the running state, and offers no Done after a failed rollout", async () => {
	const footer = mountFooter();
	const completed: IEvent[] = [];
	const deployed: boolean[] = [];
	const saved = eventRecord(APPS[APP].events[0]) as unknown as IEvent;
	const view = await mountPanel({
		footerContainer: footer,
		onCreate: async () => saved,
		onComplete: (event) => completed.push(event),
		onDeployedChange: (value) => deployed.push(value),
	});
	await click(checkbox("edge-berlin-01"));
	await click(createButton());
	await view.settle();
	expect(byRole("button", "Review deployment", footer)).toBeTruthy();
	expect(footer.textContent).not.toContain("Deploy Check-in page");
	await click(byRole("button", "Review deployment", footer));
	await view.settle();
	const blocked = byRole(
		"button",
		"Deploy Check-in page to edge-berlin-01",
		footer,
	) as HTMLButtonElement;
	expect(blocked.disabled).toBe(true);
	expect(footer.textContent).toContain("Open required settings");
	expect(view.container.querySelector("[data-deploy-action]")).toBeNull();
	await click(byRole("button", "Open required settings", footer));
	await view.settle();
	expect(byRole("tab", "Access & cost").getAttribute("aria-selected")).toBe(
		"true",
	);
	await click(byRole("checkbox", /I own Visitor Check-in/));
	await click(byRole("tab", "Review"));
	await view.settle();
	const ready = byRole(
		"button",
		"Deploy Check-in page to edge-berlin-01",
		footer,
	) as HTMLButtonElement;
	expect(ready.disabled).toBe(false);
	await click(ready);
	await view.settle();
	// The fake device refuses the upload, so nothing was deployed: Done would claim otherwise.
	expect(view.container.textContent).toContain("wasn't deployed");
	expect(footer.textContent).not.toContain("Done");
	expect(completed).toEqual([]);
	expect(deployed.at(-1)).toBe(false);
}, 30_000);

async function mountInDialog(onNavigate: (href: string) => void) {
	const { Dialog, DialogContent, DialogDescription, DialogTitle } =
		await import("../../ui/dialog");
	const controls = { close: () => {} };
	function Host({ overrides }: { overrides: DeviceWorkspaceOverrides }) {
		const [open, setOpen] = useState(true);
		controls.close = () => setOpen(false);
		return (
			<Dialog open={open}>
				<DialogContent>
					<DialogTitle>New event</DialogTitle>
					<DialogDescription>Create an event</DialogDescription>
					<NewEventDeployment
						appId={APP}
						draftEvent={draft}
						overrides={overrides}
						onNavigate={onNavigate}
						onCreate={async () => {
							throw new Error("Unexpected creation");
						}}
					/>
				</DialogContent>
			</Dialog>
		);
	}
	const view = await mountDevices(
		({ overrides }) => <Host overrides={overrides} />,
		{
			providers: false,
		},
	);
	const closeDialog = async () => {
		await act(async () => controls.close());
		await view.settle();
	};
	return { view, closeDialog };
}

async function openDiagnose() {
	await click(byRole("button", /^All/));
	await click(byRole("button", "Fix warehouse-pi"));
	await click(byRole("button", /Diagnose/));
	return byRole("dialog", /Diagnose|warehouse-pi/);
}

test("a sheet's links go to the host instead of loading a page, and closing the sheet then the dialog gives the page back", async () => {
	const hrefs: string[] = [];
	const { view, closeDialog } = await mountInDialog((href) => hrefs.push(href));
	const sheet = await openDiagnose();
	await click(byRole("link", "Open warehouse-pi", sheet));
	await view.settle();
	expect(hrefs).toHaveLength(1);
	expect(hrefs[0]).toContain(SAMPLE_IDS.warehouse);
	expect(queryByRole("dialog", /Diagnose/)).toBeNull();
	expect(document.body.style.pointerEvents).toBe("none");
	await closeDialog();
	expect(document.body.style.pointerEvents).not.toBe("none");
}, 30_000);

test("closing the dialog while a sheet is open gives the page back", async () => {
	const { closeDialog } = await mountInDialog(() => {});
	await openDiagnose();
	await closeDialog();
	expect(document.body.style.pointerEvents).not.toBe("none");
}, 30_000);

test("a signed-out account is told in the footer why events cannot be created for devices", async () => {
	const footer = mountFooter();
	const view = await mountDevices(
		({ overrides }) => (
			<NewEventDeployment
				appId={APP}
				draftEvent={draft}
				overrides={overrides}
				footerContainer={footer}
				onCreate={async () => {
					throw new Error("Unexpected creation");
				}}
			/>
		),
		{ providers: false, signedIn: false },
	);
	await view.settle();
	expect(footer.textContent).toContain("Sign in to create events for devices.");
});

test("an event the hub cannot hand to devices is flagged before it is saved", async () => {
	const footer = mountFooter();
	let creates = 0;
	const view = await mountDevices(
		({ fake, overrides }) => {
			fake.hub.capabilities.eventTypes = false;
			return (
				<NewEventDeployment
					appId={APP}
					draftEvent={draft}
					overrides={overrides}
					footerContainer={footer}
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
	await view.settle();
	expect(footer.textContent).toContain(
		"This hub can't hand forms and quick actions to devices yet.",
	);
	expect(createButton().disabled).toBe(true);
	expect((byRole("button", "Create only") as HTMLButtonElement).disabled).toBe(
		true,
	);
	expect(creates).toBe(0);
});

test("a latched flag stays true after its source turns false and starts false again on a new mount", async () => {
	const seen: boolean[] = [];
	function Probe({ value }: { value: boolean }) {
		seen.push(useLatched(value));
		return null;
	}
	const view = await dom.render(<Probe value={false} />);
	await view.rerender(<Probe value={true} />);
	await view.rerender(<Probe value={false} />);
	expect(seen.at(-1)).toBe(true);
	await view.unmount();
	seen.length = 0;
	await dom.render(<Probe value={false} />);
	expect(seen.at(-1)).toBe(false);
});

test("Pick all selects every ready device of the filter, and a long list asks once before opening that many sessions", async () => {
	await mountPanel({}, fleetWith(58, 0));
	await click(byRole("button", "Pick all 60 ready devices"));
	expect(queryByRole("button", /^Pick all/)).toBeNull();
	expect(document.body.textContent).toContain("Pick 60 devices?");
	expect(byRole("button", /^0 selected/)).toBeTruthy();
	await click(byRole("button", "Cancel"));
	expect(queryByRole("button", "Pick all 60 ready devices")).not.toBeNull();
	await click(byRole("button", "Pick all 60 ready devices"));
	await click(byRole("button", "Pick 60"));
	expect(byRole("button", /^60 selected/)).toBeTruthy();
	expect(createButton().textContent).toBe("Create & deploy to 60 devices");
});

test("selected chips keep a readable text colour", async () => {
	await mountPanel({}, fleetWith(2, 0));
	await click(byRole("button", "Pick all 4 ready devices"));
	const chip = allByRole(
		"button",
		/^Remove /,
		document.querySelector('[aria-label="Selected devices"]') as HTMLElement,
	)[0];
	expect(chip?.className).toContain("text-foreground");
	expect(chip?.className).not.toContain("text-primary");
});

test("a host-disabled form gets a reason in the footer next to both disabled buttons", async () => {
	const footer = mountFooter();
	await mountPanel({ footerContainer: footer, disabled: true });
	await click(checkbox("edge-berlin-01"));
	expect(footer.textContent).toContain("Complete the event details first.");
	expect(createButton().disabled).toBe(true);
	expect(
		(byRole("button", "Create only", footer) as HTMLButtonElement).disabled,
	).toBe(true);
});

test("the saved banner is a status region", async () => {
	const saved = eventRecord(APPS[APP].events[0]) as unknown as IEvent;
	const view = await mountPanel({ onCreate: async () => saved });
	await click(checkbox("edge-berlin-01"));
	await click(createButton());
	await view.settle();
	const heading = byRole("heading", "Check-in page saved");
	expect(heading.closest("output")).not.toBeNull();
});
