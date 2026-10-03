import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act, useEffect } from "react";
import { ApiResponseError } from "../../../../lib/api-error";
import {
	SAMPLE_IDS,
	SAMPLE_ME,
	SAMPLE_NOW,
	deviceRow,
	emptyInput,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";
import type { DeviceSeed } from "../testing/fake-device-api";
import type { FakeWorkspaceOptions } from "../testing/fake-workspace";
import type { EventsDevicesProviderProps } from "./events-devices";
import type { SampleApp } from "./events-test-kit";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { EventsDevicesProvider } = await import("./events-devices");
const { RunsOnCell, runsOnReasonId } = await import("./runs-on-cell");
const { EVENTS_BLOCK_ID, EventsDevicesBanner, OnDevicesStrip } = await import(
	"./on-devices-strip"
);
const { sampleEvents } = await import("./events-test-kit");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { serveNightlyOnEdge } = await import("../testing/schedule-scenarios");
const { readExistingDeployment } = await import(
	"../../../../lib/device-management/deployment"
);

afterEach(async () => {
	await cleanupDevices();
	useOverlayStore.getState().close();
	await dom.cleanup();
});
afterAll(dom.restore);

const APP: SampleApp = "app_invoice_ai";
const SERVED = "evt_extract_http";
const NOT_SERVED = "evt_gpu_extract";
const CANT = "evt_invoice_inbox";
const CANT_TEXT = "Can't run on devicesHandled by the hub";
const LATEST = "evt_invoice_review";
const SCHEDULE = "evt_invoice_reconcile";
/** Read boards and Read events, nothing else: sees the app's events and may not edit them. */
const READER_ROLE = {
	role_id: "role_reader",
	role_name: "Reader",
	permissions: 0b1_0001_0000_0000,
	is_owner: false,
	can_leave: true,
};
const HUB_OFF = { standalone: { enabled: false } } as never;
const HUB_ON = { standalone: { enabled: true } } as never;

let mounts = 0;
function MountCounter() {
	useEffect(() => {
		mounts += 1;
	}, []);
	return null;
}

interface MountOptions extends FakeWorkspaceOptions {
	seed?: DeviceSeed;
	signedIn?: boolean;
	props?: Partial<EventsDevicesProviderProps>;
	/** Runs on the fake hub before the page mounts. */
	before?: (fake: Awaited<ReturnType<typeof createFakeWorkspace>>) => void;
	/** The viewer's role on the app, when it is not the owner's. */
	role?: typeof READER_ROLE;
	/** A fake hub and fleet that a scenario already changed. */
	fake?: Awaited<ReturnType<typeof createFakeWorkspace>>;
}

/** The Events device column over the fake hub, with what the hub had seen before the page mounted. */
async function mountPage(appId: SampleApp, options: MountOptions = {}) {
	const {
		seed,
		signedIn,
		props,
		before,
		role,
		fake: given,
		...workspace
	} = options;
	const fake = given ?? (await createFakeWorkspace(seed, workspace));
	before?.(fake);
	const calls = fake.api.calls.length;
	const commands = fake.api.commands.length;
	const events = sampleEvents(appId);
	mounts = 0;
	const view = await mountDevices(
		({ overrides }) => (
			<EventsDevicesProvider
				appId={appId}
				events={events}
				canReadBoards
				harness={{ overrides }}
				{...props}
			>
				<MountCounter />
				<EventsDevicesBanner />
				<OnDevicesStrip />
				{events.map((event) => (
					<RunsOnCell key={event.id} appId={appId} eventId={event.id} />
				))}
			</EventsDevicesProvider>
		),
		{
			fake,
			providers: false,
			signedIn,
			...(role
				? {
						backend: {
							roleState: { getOwnRole: async () => role },
						} as never,
					}
				: {}),
		},
	);
	const cell = (eventId: string) => {
		const found = document.getElementById(runsOnReasonId(eventId));
		if (!found) throw new Error(`no cell for ${eventId}`);
		return found;
	};
	return {
		...view,
		cell,
		text: (eventId: string) => cell(eventId).textContent,
		banner: () => document.getElementById(EVENTS_BLOCK_ID),
		strip: () => view.container.querySelector<HTMLElement>("[data-on-devices]"),
		/** Hub requests and agent commands sent since the page mounted. */
		since: () => ({
			calls: fake.api.calls.slice(calls),
			commands: fake.api.commands.slice(commands),
		}),
	};
}

/** One owned device this computer has no keys for: it can't take a deploy. */
function keylessFleet(): DeviceSeed {
	const seed = emptyInput();
	seed.hub = { state: "on", serverTime: SAMPLE_NOW };
	seed.devices.push(
		deviceRow({
			device_id: "11111111-2222-4333-8444-555555555555",
			owner_id: SAMPLE_ME,
			name: "keyless-box",
			status: "active",
			registered_at: SAMPLE_NOW - 30 * 86_400,
			last_seen_at: SAMPLE_NOW - 30,
			relationship: "owner",
		}),
	);
	return seed;
}

describe("EventsDevicesProvider over the device workspace", () => {
	test("reads the fleet once for every row and only reads", async () => {
		const view = await mountPage(APP);
		const chip = byRole("link", "edge-berlin-01", view.cell(SERVED));
		expect(chip.getAttribute("href")).toBe(
			`/library/config/devices?id=${APP}&device=${SAMPLE_IDS.edge}&service=invoice-extractor`,
		);
		expect(view.text(NOT_SERVED)).toContain("Not on a device you can see");
		expect(view.text(CANT)).toBe(CANT_TEXT);
		expect(view.strip()?.textContent).toContain(
			"5 of 6 events can run on a device. 1 of them runs on 1 device you can see.",
		);
		expect(view.banner()).toBeNull();
		expect(
			view.fake.api
				.writes()
				.filter((call) => view.since().calls.includes(call)),
		).toEqual([]);
		expect(view.since().commands).toEqual([]);
	});

	test("the rows mount once: the workspace arrives next to them, not around them", async () => {
		await mountPage(APP);
		expect(mounts).toBe(1);
	});

	test("signed out: no workspace, no hub request, one banner with Sign in", async () => {
		const view = await mountPage(APP, { signedIn: false, unlock: "none" });
		expect(view.text(SERVED)).toBe("Sign in to see devices");
		expect(view.strip()).toBeNull();
		expect(view.since().calls).toEqual([]);
		await click(byRole("button", "Sign in"));
		expect(view.signIns()).toBe(1);
	});

	test("hub-off from the host's hub record: no workspace is created and nothing is requested", async () => {
		const view = await mountPage(APP, {
			unlock: "none",
			props: { hub: HUB_OFF },
		});
		expect(view.text(SERVED)).toBe("Device status off on this hub");
		expect(view.text(CANT)).toBe("Device status off on this hub");
		expect(view.banner()?.textContent).toContain(
			"Device support is off on this hub",
		);
		expect(view.strip()).toBeNull();
		expect(view.since().calls).toEqual([]);
	});

	test("hub-off found by the workspace when the host has no hub record yet", async () => {
		const view = await mountPage(APP, {
			unlock: "none",
			before: (fake) => {
				fake.api.mode.devicesEnabled = false;
			},
		});
		expect(view.text(SERVED)).toBe("Device status off on this hub");
		expect(view.banner()?.getAttribute("data-events-block")).toBe("hub_off");
		expect(view.strip()).toBeNull();
	});

	test("no ReadBoards: no device read is sent; events that can't run still say why", async () => {
		const view = await mountPage(APP, {
			unlock: "none",
			props: { hub: HUB_ON, canReadBoards: false },
		});
		expect(view.text(SERVED)).toBe("Unknown: you can't read this app's flows");
		expect(view.text(CANT)).toBe(CANT_TEXT);
		expect(view.banner()).toBeNull();
		expect(view.strip()).toBeNull();
		expect(view.since().calls).toEqual([]);
	});

	test("a token that can't manage devices blocks with its own cause", async () => {
		const view = await mountPage(APP, {
			unlock: "none",
			before: (fake) => {
				fake.api.mode.tokenRestricted = true;
			},
		});
		expect(view.banner()?.getAttribute("data-events-block")).toBe("token");
		expect(view.text(SERVED)).toBe("Sign in to see devices");
	});

	test("a refresh that fails after a good read keeps the cells and says so in the strip's stamp (R5)", async () => {
		const view = await mountPage(APP);
		view.fake.api.fail(
			{ method: "GET", path: "devices" },
			new ApiResponseError({
				status: 429,
				code: "RATE_LIMITED",
				message: "Too many requests",
			}),
		);
		await act(async () => {
			await view.fake.queryClient.refetchQueries();
		});
		await view.settle();
		expect(byRole("link", "edge-berlin-01", view.cell(SERVED))).toBeTruthy();
		expect(view.banner()).toBeNull();
		expect(
			view.strip()?.querySelector("[data-stamp][data-age='error']"),
		).toBeTruthy();
		expect(view.strip()?.textContent).toContain(
			"5 of 6 events can run on a device.",
		);
	});

	test("the device list can't be read: never “Not on a device”, one banner, and Retry recovers", async () => {
		let stop: () => void = () => undefined;
		const view = await mountPage(APP, {
			before: (fake) => {
				// A refusal is a verdict: the list isn't retried, so the result is immediate.
				stop = fake.api.fail(
					{ method: "GET", path: "devices" },
					new ApiResponseError({
						status: 429,
						code: "RATE_LIMITED",
						message: "Too many requests",
					}),
				);
			},
		});
		expect(view.text(NOT_SERVED)).toBe("Device status unavailable");
		expect(view.container.textContent).not.toContain("Not on a device");
		expect(view.banner()?.textContent).toContain("Devices couldn't be checked");
		expect(view.banner()?.textContent).toContain(
			"The hub is limiting requests.",
		);
		expect(view.strip()).toBeNull();
		stop();
		await click(byRole("button", "Retry"));
		await view.settle();
		expect(view.banner()).toBeNull();
		expect(byRole("link", "edge-berlin-01", view.cell(SERVED))).toBeTruthy();
	});

	test("no device can take a deploy: one banner, not one reason per row", async () => {
		const view = await mountPage(APP, { seed: keylessFleet() });
		expect(view.banner()?.textContent).toContain(
			"You have no device that can take a deploy right now",
		);
		expect(document.querySelectorAll("[data-events-block]")).toHaveLength(1);
		expect(queryByRole("link", "Run on a device…")).toBeNull();
	});

	test("locked devices still count as deploy targets: no banner, Unlock N… in the strip", async () => {
		const view = await mountPage(APP, { unlock: "none" });
		expect(view.banner()).toBeNull();
		expect(view.text(NOT_SERVED)).toContain("Not on a device you can see");
		expect(
			byRole("button", /^Unlock \d…$/, view.strip() ?? undefined),
		).toBeTruthy();
		expect(
			byRole("link", "Run on a device…", view.cell(NOT_SERVED)),
		).toBeTruthy();
	});

	test("older hub: the column works without the app placements route and asks for it once", async () => {
		const view = await mountPage(APP, { hubVersion: "old" });
		expect(view.banner()).toBeNull();
		expect(byRole("link", "edge-berlin-01", view.cell(SERVED))).toBeTruthy();
		expect(view.strip()?.querySelector("[data-age='error']")).toBeNull();
		expect(
			view.fake.api.sent("GET", /device-placements/).length,
		).toBeLessThanOrEqual(1);
		// What the one shared device's access covers is asked once, never retried on a hub without the route.
		expect(view.fake.api.sent("GET", /my-access/).length).toBeLessThanOrEqual(
			1,
		);
	});

	test("older agent: status without event lists reads unknown, and no command is sent", async () => {
		const view = await mountPage(APP, { agentFeatures: {} });
		expect(view.banner()).toBeNull();
		expect(view.text(SERVED)).toContain("Not on a device you can see");
		expect(byRole("button", /unknown$/, view.cell(SERVED))).toBeTruthy();
		expect(view.strip()?.textContent).toContain(
			"None of them is on a device you can see yet.",
		);
		await click(
			byRole("button", "Where Extract invoice runs", view.cell(SERVED)),
		);
		expect(
			byRole("dialog", "Where Extract invoice runs").textContent,
		).toContain("its status doesn't list the events it serves");
		expect(view.since().commands).toEqual([]);
	});

	test("Unlock… from the strip opens the unlock sheet outside the Devices area", async () => {
		const view = await mountPage(APP);
		await click(
			byRole("button", "Unlock lab-gpu-02…", view.strip() ?? undefined),
		);
		await view.settle();
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: SAMPLE_IDS.lab,
		});
		expect(byRole("dialog").textContent).toContain("lab-gpu-02");
	});

	test("Online or offline? opens the comparison for this app", async () => {
		const view = await mountPage(APP);
		await click(
			byRole("button", "Online or offline?", view.strip() ?? undefined),
		);
		expect(byRole("dialog").textContent).toContain(
			"Invoice AI is an online app",
		);
	});

	test("a device shared for another app is 'no access': not locked, not unknown, no Unlock…", async () => {
		// lab-gpu-02 is shared for Invoice AI only: unlocking it can never show CRM Sync.
		const view = await mountPage("app_crm_sync");
		const strip = view.strip();
		expect(strip?.textContent).toContain(
			"your access doesn't cover CRM Sync on 1 more",
		);
		expect(queryByRole("button", /^Unlock/, strip ?? undefined)).toBeNull();
		const cell = view.cell("evt_crm_webhook");
		expect(cell.textContent).toContain("Not on a device");
		expect(cell.textContent).not.toContain("you can see");
		expect(cell.querySelector("[data-runs-on-unknown]")).toBeNull();
		await click(byRole("button", "Where CRM webhook runs", cell));
		const dialog = byRole("dialog", "Where CRM webhook runs");
		expect(dialog.textContent).not.toContain("Status unknown");
		expect(
			dialog
				.querySelector(`[data-on-device="${SAMPLE_IDS.lab}"]`)
				?.getAttribute("data-state"),
		).toBe("no_access");
	});

	test("a device that never checked in: 'hasn't checked in yet' with Start instructions, never Unlock…", async () => {
		const view = await mountPage(APP);
		await click(
			byRole(
				"button",
				"Where Extract invoice (GPU) runs",
				view.cell(NOT_SERVED),
			),
		);
		const dialog = byRole("dialog", "Where Extract invoice (GPU) runs");
		const row = dialog.querySelector<HTMLElement>(
			`[data-on-device="${SAMPLE_IDS.cold}"]`,
		);
		if (!row) throw new Error("no row for cold-storage-nas");
		expect(row.textContent).toContain(
			"cold-storage-nas hasn't checked in yet. Deploying needs it online first.",
		);
		expect(queryByRole("button", /Unlock/, row)).toBeNull();
		expect(byRole("link", "Start instructions", row).getAttribute("href")).toBe(
			`/library/config/devices?id=${APP}&device=${SAMPLE_IDS.cold}&tab=overview`,
		);
	});
});

describe("RunsOnCell inside the Devices area (App › Devices, list mode)", () => {
	const service = `/library/config/devices?id=${APP}&device=${SAMPLE_IDS.edge}&service=invoice-extractor`;

	test("without an Events page around it, a cell reads the area's workspace and uses the area's route", async () => {
		const view = await mountDevices(
			<RunsOnCell appId={APP} eventId={SERVED} />,
			{ host: "app", search: `id=${APP}&by=event` },
		);
		const chip = byRole("link", "edge-berlin-01", view.container);
		expect(chip.getAttribute("href")).toBe(service);
		expect(view.container.textContent).toContain("1 runs an older version");
		await click(chip);
		expect(view.navigations.at(-1)).toEqual({ mode: "push", href: service });
	});

	test("its popover opens there too and its links stay in the area", async () => {
		const view = await mountDevices(
			<RunsOnCell appId={APP} eventId={NOT_SERVED} />,
			{ host: "app", search: `id=${APP}&by=event` },
		);
		await click(byRole("button", "Where Extract invoice (GPU) runs"));
		const dialog = byRole("dialog", "Where Extract invoice (GPU) runs");
		expect(dialog.textContent).toContain("Status unknown · 1");
		await click(byRole("link", "Run on a device…", dialog));
		expect(view.navigations.at(-1)?.href).toBe(
			`/library/config/devices?id=${APP}&flow=deploy&mode=new&event=${NOT_SERVED}&from=events`,
		);
	});

	test("an event that can't run says why without an Events page", async () => {
		const view = await mountDevices(<RunsOnCell appId={APP} eventId={CANT} />, {
			host: "app",
			search: `id=${APP}&by=event`,
		});
		expect(view.container.textContent).toBe(CANT_TEXT);
	});
});

describe("schedules and Latest events over the hub", () => {
	const where = () =>
		document.querySelector<HTMLElement>("[data-schedule-where]");

	async function openSchedule(options: MountOptions = {}) {
		const view = await mountPage(APP, options);
		await click(
			byRole(
				"button",
				"Where Nightly reconciliation runs",
				view.cell(SCHEDULE),
			),
		);
		return {
			...view,
			dialog: byRole("dialog", "Where Nightly reconciliation runs"),
		};
	}

	test("a schedule nobody moved runs on the hub, and can run on a device", async () => {
		const view = await openSchedule();
		expect(where()?.getAttribute("data-schedule-where")).toBe("hub");
		expect(where()?.textContent).toContain("Runs on the hub.");
		expect(
			byRole("link", "Run on a device…", view.cell(SCHEDULE)),
		).toBeTruthy();
	});

	test("an older hub can't hand schedules to devices: the schedule says so, other events are unaffected", async () => {
		const view = await mountPage(APP, { hubVersion: "old" });
		expect(view.text(SCHEDULE)).toBe(
			"Can't run on devicesHub can't run schedules on devices yet",
		);
		expect(byRole("link", "edge-berlin-01", view.cell(SERVED))).toBeTruthy();
	});

	test("released to a service: Run it on the hub again asks the hub once and the list follows", async () => {
		const view = await openSchedule({
			before: (fake) => {
				fake.hub.schedules.release(
					APP,
					SCHEDULE,
					SAMPLE_IDS.edge,
					"invoice-extractor",
				);
			},
		});
		expect(where()?.textContent).toContain(
			"Moves to edge-berlin-01 › invoice-extractor when that service starts it. The hub runs it until then.",
		);
		await click(byRole("button", "Run it on the hub again", view.dialog));
		await click(
			byRole(
				"button",
				"Run it on the hub again",
				byRole("region", "Run it on the hub again", view.dialog),
			),
		);
		await view.settle();
		expect(
			view.fake.api.sent("DELETE", /device-schedules/).map(([, path]) => path),
		).toEqual([`apps/${APP}/device-schedules/${SCHEDULE}`]);
		expect(where()?.getAttribute("data-schedule-where")).toBe("hub");
		expect(where()?.textContent).toContain(
			"The hub runs Nightly reconciliation.",
		);
		expect(view.since().commands).toEqual([]);
	});

	test("one place, end to end: deployed it runs on the device, stopped it runs nowhere, given back it returns to the hub", async () => {
		const fake = await createFakeWorkspace();
		expect(await serveNightlyOnEdge(fake)).toBeNull();
		const view = await openSchedule({ fake });
		expect(where()?.getAttribute("data-schedule-where")).toBe("device");
		expect(where()?.textContent).toContain(
			"Runs on edge-berlin-01, not on the hub.",
		);
		// No other device can take it while edge-berlin-01 runs it.
		expect(view.dialog.textContent).toContain(
			"Another service runs this schedule. A schedule runs in one place.",
		);

		await act(async () => {
			await fake.workspace.live.call(SAMPLE_IDS.edge)({
				type: "stop",
				placement_id: "invoice-extractor",
			});
			fake.api.hub.publishStatus(
				SAMPLE_IDS.edge,
				fake.api.agent(SAMPLE_IDS.edge),
			);
			await fake.workspace.live.refreshInspection(SAMPLE_IDS.edge);
		});
		await view.settle();
		expect(where()?.getAttribute("data-schedule-where")).toBe("device_idle");
		expect(where()?.textContent).toContain(
			"but nothing runs it: the service is stopped.",
		);
		expect(fake.hub.schedules.listing(APP)).toMatchObject([
			{ event_id: SCHEDULE, state: "device", device_id: SAMPLE_IDS.edge },
		]);

		await click(byRole("button", "Run it on the hub again", view.dialog));
		await click(
			byRole(
				"button",
				"Run it on the hub again",
				byRole("region", "Run it on the hub again", view.dialog),
			),
		);
		await view.settle();
		expect(
			fake.api.sent("DELETE", /device-schedules/).map(([, path]) => path),
		).toEqual([`apps/${APP}/device-schedules/${SCHEDULE}`]);
		expect(fake.hub.schedules.listing(APP)).toMatchObject([
			{ event_id: SCHEDULE, state: "returning" },
		]);
		expect(where()?.getAttribute("data-schedule-where")).toBe("returning");
	});

	test("an update that drops the schedule hands it back: it returns to the hub after the grace period", async () => {
		const fake = await createFakeWorkspace();
		expect(await serveNightlyOnEdge(fake)).toBeNull();
		await act(async () => {
			const call = fake.workspace.live.call(SAMPLE_IDS.edge);
			const existing = await readExistingDeployment(
				call,
				"invoice-extractor",
				APP,
			);
			await call({
				type: "apply",
				config: {
					...existing.config,
					events: existing.config.events.filter(
						(event) => event.event_id !== SCHEDULE,
					),
				},
				expected_revision: existing.config_revision,
				start: true,
			});
		});
		expect(fake.hub.schedules.listing(APP)).toMatchObject([
			{ event_id: SCHEDULE, state: "returning" },
		]);
		await openSchedule({ fake });
		expect(where()?.getAttribute("data-schedule-where")).toBe("returning");
		// The service retired its process before it handed the schedule back: 5 minutes.
		expect(where()?.textContent).toContain("Returns to the hub at 12:05.");
	});

	test("a role that can't edit the app's events: the way back is shown, off, with who can", async () => {
		const view = await openSchedule({
			role: READER_ROLE,
			before: (fake) => {
				fake.hub.schedules.release(
					APP,
					SCHEDULE,
					SAMPLE_IDS.edge,
					"invoice-extractor",
				);
			},
		});
		const back = byRole("button", "Run it on the hub again", view.dialog);
		expect(back.getAttribute("aria-disabled")).toBe("true");
		expect(view.dialog.textContent).toContain(
			"Only someone who can edit this app's events can move it.",
		);
		await click(back);
		expect(view.fake.api.sent("DELETE", /device-schedules/)).toEqual([]);
	});

	test("an event that follows Latest reads its flow's state from the hub and can run on a device", async () => {
		const view = await mountPage(APP);
		expect(byRole("link", "Run on a device…", view.cell(LATEST))).toBeTruthy();
		expect(
			view.fake.api.sent("GET", /version\/current/).map(([, path]) => path),
		).toEqual([`apps/${APP}/board/flow_review/version/current`]);
	});

	test("an older hub can't deploy Latest: the event says so and points at a flow version", async () => {
		const view = await mountPage(APP, { hubVersion: "old" });
		expect(view.text(LATEST)).toBe(
			"Can't run on devicesHub can't deploy Latest yet",
		);
		await click(
			byRole(
				"button",
				"Why Review queue can't run on devices",
				view.cell(LATEST),
			),
		);
		expect(
			byRole("dialog", "Why Review queue can't run on devices").textContent,
		).toContain(
			"This hub can't deploy events that follow Latest yet. Update the hub, or pin a flow version in Events.",
		);
	});
});
