import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	SAMPLE_PEOPLE,
	sampleFleet,
	sampleFleetOlderHub,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { generateFleet } from "../../../../lib/device-management/model/__fixtures__/sample-fleet-200";
import {
	advance,
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspaceOptions } from "../testing/fake-workspace";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { FleetScreen } = await import("./fleet-screen");
const { laneTicksOf } = await import("./fleet-device-row");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useOverlayStore } = await import("../workspace/overlay-store");

async function resetAfterTest() {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
}
afterEach(resetAfterTest);
afterAll(dom.restore);

/** A 200-device fleet takes several seconds to seed and mount on a busy machine. */
const SCALE_TIMEOUT_MS = 30_000;

/** The account directory of the fake: the sample people by account id. */
function personName(id: string): string | undefined {
	const people: Record<string, string> = {
		[SAMPLE_PEOPLE.felix]: "Felix Schultz",
		[SAMPLE_PEOPLE.mira]: "Mira Novak",
		[SAMPLE_PEOPLE.partner]: "Partner Ops",
	};
	return people[id];
}

function Routed() {
	const { route, scope } = useDevicesRoute();
	return <FleetScreen route={route} scope={scope} />;
}

/** The screen over a fake whose account directory knows the sample people. */
async function mountFleet(
	options: MountDevicesOptions & FakeWorkspaceOptions = {},
) {
	const fake = await createFakeWorkspace(options.seed, options);
	const userState = {
		async getProfile() {
			return fake.profile;
		},
		async getInfo() {
			return { id: fake.hub.me, dev_mode: false };
		},
		async lookupUser(id: string) {
			return person(id);
		},
		async lookupUsers(ids: string[]) {
			return ids.filter(personName).map(person);
		},
	};
	const backend = { userState: userState as never };
	const mounted = await mountDevices(<Routed />, { ...options, fake, backend });
	// The directory answers a screenful of names in one batch, 20 ms later.
	await advance(40);
	await mounted.settle();
	return mounted;
}

function person(id: string) {
	return { id, name: personName(id), created_at: "2026-01-01T00:00:00Z" };
}

function table(container: HTMLElement) {
	return container.querySelector(
		"#devices-fleet-list table",
	) as HTMLTableElement;
}

function rowOf(container: HTMLElement, deviceId: string) {
	return container.querySelector(
		`tr[data-device="${deviceId}"]`,
	) as HTMLElement;
}

function rowIds(container: HTMLElement) {
	const ids: (string | undefined)[] = [];
	for (const row of table(container).querySelectorAll<HTMLElement>(
		"tr[data-device]",
	))
		ids.push(row.dataset.device);
	return ids;
}

function cells(row: HTMLElement) {
	const texts: string[] = [];
	for (const cell of row.querySelectorAll("td"))
		texts.push(cell.textContent ?? "");
	return texts;
}

/** The group header rows of the table, in order. */
function groupTexts(container: HTMLElement) {
	const texts: string[] = [];
	for (const row of table(container).querySelectorAll("tr[data-group]"))
		texts.push(row.textContent ?? "");
	return texts;
}

describe("FleetDevicesTable rows", () => {
	test("eight columns in the spec order, one row per active device, most severe first", async () => {
		const { container } = await mountFleet();
		expect(
			allByRole("columnheader", undefined, table(container)).map((header) =>
				(header.textContent ?? "").trim(),
			),
		).toEqual([
			"Device",
			"Health",
			"Check-inlast 24 h",
			"Services",
			"Certificates",
			"Access & keys",
			"Agent",
			"More",
		]);
		expect(rowIds(container)).toEqual([
			SAMPLE_IDS.warehouse,
			SAMPLE_IDS.edge,
			SAMPLE_IDS.studio,
			SAMPLE_IDS.cold,
			SAMPLE_IDS.lab,
		]);
		expect(container.querySelector("[data-fleet-count]")?.textContent).toBe(
			"Showing 7 of 7",
		);
	});

	test("each cell states what its plane knows: hub facts without keys, the rest only when readable", async () => {
		const { container } = await mountFleet();
		const [device, health, checkin, services, certificates, access, agent] =
			cells(rowOf(container, SAMPLE_IDS.warehouse));
		expect(device).toContain("warehouse-pi");
		expect(device).toContain("54484ac9");
		expect(device).toContain("registered");
		expect(health).toContain("Critical");
		expect(health).toContain("Crashing");
		expect(checkin).toContain("Offline since");
		expect(services).toContain("1 crashing");
		expect(services).toContain("Last known");
		expect(certificates).toContain("Warning");
		expect(certificates).toContain("1 · expired");
		expect(certificates).toContain("confirmed 3h ago");
		expect(access).toContain("Yours");
		expect(access).toContain("Unlocked");
		expect(agent).toContain("0.9.2");
		expect(agent).toContain("read");

		const lab = cells(rowOf(container, SAMPLE_IDS.lab));
		expect(lab[1]).toContain("Status unknown");
		expect(lab[1]).toContain("Locked on this computer");
		expect(lab[2]).toContain("Online");
		expect(lab[3]).toBe("Lockedunlock to read");
		expect(lab[4]).toBe("No accessneeds whole-device View status");
		expect(lab[5]).toMatch(/^Shared by Mira Novak · ends in \d+/);
		expect(lab[6]).toContain("Locked");

		const cold = cells(rowOf(container, SAMPLE_IDS.cold));
		expect(cold[2]).toContain("Never checked in");
		expect(cold[2]).toContain("registered 12m ago");
		expect(cold[3]).toBe("Not loadedno status yet");
		expect(cold[4]).toContain("Not reported");

		const edge = cells(rowOf(container, SAMPLE_IDS.edge));
		expect(edge[3]).toContain("3 · 1 as requested · 1 updating · 1 stopped");
		expect(edge[4]).toContain("2 · next expires in 5d");
		expect(edge[5]).toContain("Live · relayed");
		expect(edge[6]).toContain("Linux");
	});

	test("certificate warnings come from the hub and show while every device is locked", async () => {
		const { container } = await mountFleet({ unlock: "none" });
		const edge = cells(rowOf(container, SAMPLE_IDS.edge));
		expect(edge[3]).toBe("Lockedunlock to read");
		expect(edge[4]).toContain("Warning");
		expect(edge[4]).toContain("2 · next expires in 5d");
		expect(cells(rowOf(container, SAMPLE_IDS.warehouse))[4]).toContain(
			"1 · expired",
		);
	});

	test("older hub: each row reads its own certificate list, once", async () => {
		const { fake, container } = await mountFleet({
			seed: sampleFleetOlderHub(),
			hubVersion: "old",
		});
		expect(cells(rowOf(container, SAMPLE_IDS.edge))[4]).toContain(
			"2 · next expires in 5d",
		);
		expect(cells(rowOf(container, SAMPLE_IDS.warehouse))[4]).toContain(
			"1 · expired",
		);
		expect(cells(rowOf(container, SAMPLE_IDS.lab))[5]).toContain(
			"Shared or cloud approvals",
		);
		for (const id of [SAMPLE_IDS.edge, SAMPLE_IDS.warehouse, SAMPLE_IDS.lab])
			expect(
				fake.api.sent("GET", `devices/${id}/certificate-inventory`).length,
			).toBe(1);
		expect(queryByRole("alert", undefined, container)).toBeNull();
	});

	test("a status reader without the service list is never shown an empty list; an authorised empty one is", async () => {
		const metricsOnly = sampleFleet();
		const warehouse = metricsOnly.fleet[SAMPLE_IDS.warehouse];
		warehouse.status = undefined;
		warehouse.saved = undefined;
		const first = await mountFleet({ seed: metricsOnly });
		const unread = cells(rowOf(first.container, SAMPLE_IDS.warehouse))[3];
		expect(unread).not.toContain("No services");
		expect(unread).toContain("Not loaded");
		await cleanupDevices();

		const empty = sampleFleet();
		const state = empty.fleet[SAMPLE_IDS.warehouse];
		for (const observation of state.status?.observations ?? [])
			observation.placements = [];
		state.saved = undefined;
		const second = await mountFleet({ seed: empty });
		const read = cells(rowOf(second.container, SAMPLE_IDS.warehouse))[3];
		expect(read).toContain("No services");
		expect(read).not.toContain("Not loaded");
	});

	test("the lane claims only what the last check-in proves", () => {
		const now = 1_000_000;
		const row = {
			status: "active" as const,
			registered_at: now - 10 * 900,
			last_seen_at: now - 3 * 900 - 10,
		};
		const ticks = laneTicksOf(row, now);
		expect(ticks.length).toBe(96);
		expect(ticks.slice(-3)).toEqual(["miss", "miss", "miss"]);
		expect(ticks.at(-4)).toBe("ok");
		expect(ticks.slice(0, 92).every((tick) => tick === "pre")).toBe(true);
		expect(
			laneTicksOf({ ...row, last_seen_at: null }, now).filter(
				(tick) => tick === "miss",
			).length,
		).toBe(11);
		expect(laneTicksOf({ ...row, status: "revoked" }, now)).toEqual([]);
	});
});

describe("FleetDevicesTable navigation and actions", () => {
	test("a row opens its device; the controls inside it don't", async () => {
		const { container, navigations } = await mountFleet();
		const row = rowOf(container, SAMPLE_IDS.edge);
		await click(byRole("button", "Copy device ID", row));
		expect(navigations).toEqual([]);
		await click(row.querySelectorAll("td")[3]);
		expect(navigations).toEqual([
			{
				mode: "push",
				href: `/settings/devices?device=${SAMPLE_IDS.edge}&tab=overview`,
			},
		]);
	});

	test("Unlock… in a locked row raises the unlock sheet and sends nothing", async () => {
		const { fake, container } = await mountFleet();
		const writes = fake.api.writes().length;
		await click(byRole("button", "Unlock…", rowOf(container, SAMPLE_IDS.lab)));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: SAMPLE_IDS.lab,
		});
		expect(fake.api.writes().length).toBe(writes);
	});

	test("the row menu offers what applies to the device and explains what doesn't", async () => {
		const { container, navigations } = await mountFleet();
		const open = async (deviceId: string, name: string) => {
			await click(
				byRole("button", `More for ${name}`, rowOf(container, deviceId)),
			);
			return inPortal("menu");
		};
		const items = (menu: HTMLElement) =>
			[...menu.querySelectorAll<HTMLElement>("[data-menu-item]")].map(
				(item) => item.dataset.menuItem,
			);

		let menu = await open(SAMPLE_IDS.edge, "edge-berlin-01");
		expect(items(menu)).toEqual([
			"open",
			"copy-id",
			"deploy",
			"lock",
			"diagnose",
			"cloud",
			"keys",
			"revoke",
		]);
		const deploy = menu.querySelector(
			'[data-menu-item="deploy"]',
		) as HTMLElement;
		expect(deploy.textContent).toBe("Deploy an app…");
		expect(deploy.getAttribute("href")).toBe(
			`/settings/devices?flow=deploy&device=${SAMPLE_IDS.edge}`,
		);
		expect(
			menu.querySelector('[data-menu-item="revoke"]')?.getAttribute("href"),
		).toBe(
			`/settings/devices?device=${SAMPLE_IDS.edge}&tab=settings&action=revoke`,
		);
		await click(menu.querySelector('[data-menu-item="diagnose"]') as Element);
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "diagnose",
			deviceId: SAMPLE_IDS.edge,
		});

		menu = await open(SAMPLE_IDS.warehouse, "warehouse-pi");
		const offline = menu.querySelector(
			'[data-menu-item="deploy"]',
		) as HTMLElement;
		expect(offline.getAttribute("aria-disabled")).toBe("true");
		expect(offline.textContent).toMatch(
			/Deploy an app…Offline since .+\. Deploying needs a live connection\./,
		);
		const before = navigations.length;
		await click(offline);
		expect(navigations.length).toBe(before);
	});

	test("a device shared for one app deploys that app", async () => {
		const { container, settle } = await mountFleet();
		await click(
			byRole("button", "More for lab-gpu-02", rowOf(container, SAMPLE_IDS.lab)),
		);
		await settle();
		const menu = inPortal("menu");
		expect(menu.querySelector('[data-menu-item="revoke"]')).toBeNull();
		expect(menu.querySelector('[data-menu-item="unlock"]')).not.toBeNull();
		const deploy = menu.querySelector(
			'[data-menu-item="deploy"]',
		) as HTMLElement;
		expect(deploy.textContent).toBe(
			"Deploy Invoice AI…Shared with you for Invoice AI only",
		);
		expect(deploy.getAttribute("href")).toBe(
			`/settings/devices?flow=deploy&device=${SAMPLE_IDS.lab}&app=app_invoice_ai`,
		);
	});

	test("a device that never checked in can't be deployed to, and says why", async () => {
		const { container } = await mountFleet();
		await click(
			byRole(
				"button",
				"More for cold-storage-nas",
				rowOf(container, SAMPLE_IDS.cold),
			),
		);
		const deploy = inPortal("menu").querySelector(
			'[data-menu-item="deploy"]',
		) as HTMLElement;
		expect(deploy.getAttribute("aria-disabled")).toBe("true");
		expect(deploy.textContent).toBe(
			"Deploy an app…It hasn't checked in yet. Deploying needs a live connection.",
		);
	});

	test("shared access without Deploy & configure says so", async () => {
		const seed = sampleFleet();
		const grant = seed.myAccess?.[SAMPLE_IDS.lab]?.grants[0];
		if (grant) grant.capabilities = ["status", "metrics"];
		const { container, settle } = await mountFleet({ seed });
		await click(
			byRole("button", "More for lab-gpu-02", rowOf(container, SAMPLE_IDS.lab)),
		);
		await settle();
		const deploy = inPortal("menu").querySelector(
			'[data-menu-item="deploy"]',
		) as HTMLElement;
		expect(deploy.getAttribute("aria-disabled")).toBe("true");
		expect(deploy.textContent).toContain(
			"Your access doesn't include Deploy & configure.",
		);
	});
});

describe("FleetDevicesTable filters, sorting and paging", () => {
	test("the health and presence filters live in the URL", async () => {
		const { container, navigations } = await mountFleet();
		const chips = byRole("group", "Filter by health, presence or keys");
		await click(byRole("button", "Offline", chips));
		expect(navigations.at(-1)).toEqual({
			mode: "replace",
			href: "/settings/devices?filter=offline",
		});
		expect(rowIds(container)).toEqual([SAMPLE_IDS.warehouse]);
		expect(
			byRole("button", "Offline", chips).getAttribute("aria-pressed"),
		).toBe("true");
		await click(byRole("button", "All", chips));
		expect(navigations.at(-1)?.href).toBe("/settings/devices");
		expect(rowIds(container).length).toBe(5);
	});

	test("a filter in the link is applied on arrival", async () => {
		const { container } = await mountFleet({ search: "filter=locked" });
		expect(rowIds(container)).toEqual([SAMPLE_IDS.cold, SAMPLE_IDS.lab]);
		expect(container.querySelector("[data-fleet-count]")?.textContent).toBe(
			"Showing 2 of 7",
		);
	});

	test("Shared with me narrows to devices that aren't yours", async () => {
		const { container, navigations } = await mountFleet();
		await click(byRole("button", "Shared with me"));
		expect(rowIds(container)).toEqual([SAMPLE_IDS.lab]);
		expect(navigations).toEqual([]);

		// A window of the annunciator takes over, and releasing it shows every device again.
		const critical = container.querySelector(
			'[data-window="critical"]',
		) as HTMLElement;
		await click(critical);
		expect(rowIds(container)).toEqual([SAMPLE_IDS.warehouse]);
		await click(critical);
		expect(rowIds(container).length).toBe(5);
	});

	test("search matches name, ID, app and service at once and reaches the URL when typing rests", async () => {
		const { container, navigations } = await mountFleet();
		const box = byRole("textbox", "Search");
		await typeInto(box, "scanner");
		expect(rowIds(container)).toEqual([SAMPLE_IDS.warehouse]);
		expect(navigations).toEqual([]);
		await advance(350);
		expect(navigations.at(-1)).toEqual({
			mode: "replace",
			href: "/settings/devices?q=scanner",
		});
		await typeInto(box, "c8ea48cc");
		expect(rowIds(container)).toEqual([SAMPLE_IDS.lab]);
		await typeInto(box, "invoice ai");
		expect(rowIds(container)).toEqual([SAMPLE_IDS.edge]);
	});

	test("nothing matching says so and clears with one button", async () => {
		const { container } = await mountFleet({ search: "q=no-such-device" });
		expect(rowIds(container)).toEqual([]);
		expect(table(container).textContent).toContain(
			"No device matches these filters",
		);
		expect(table(container).textContent).toContain("7 devices in total.");
		await click(byRole("button", "Clear filters"));
		expect(rowIds(container).length).toBe(5);
	});

	test("grouping by health or relationship puts a counted header above each group", async () => {
		const { container } = await mountFleet();
		expect(groupTexts(container).length).toBe(1);
		await click(byRole("combobox", "Group"));
		await click(byRole("option", "Group by health", inPortal("listbox")));
		const byHealth = groupTexts(container);
		expect(byHealth.slice(0, 3)).toEqual([
			"Critical1",
			"Needs attention3",
			"Status unknown1",
		]);
		expect(byHealth[3]).toStartWith("Revoked · 2");
		expect(rowIds(container).length).toBe(5);

		await click(byRole("combobox", "Group"));
		await click(byRole("option", "Group by relationship", inPortal("listbox")));
		expect(groupTexts(container).slice(0, 2)).toEqual([
			"Yours4",
			"Shared with you1",
		]);
	});

	test("the column headers sort", async () => {
		const { container } = await mountFleet();
		const header = (name: RegExp) =>
			byRole(
				"button",
				name,
				table(container).querySelector("thead") as Element,
			);
		expect(
			header(/^Health/)
				.closest("th")
				?.getAttribute("aria-sort"),
		).toBe("descending");
		await click(header(/^Device/));
		expect(
			header(/^Device/)
				.closest("th")
				?.getAttribute("aria-sort"),
		).toBe("ascending");
		expect(rowIds(container)).toEqual([
			SAMPLE_IDS.cold,
			SAMPLE_IDS.edge,
			SAMPLE_IDS.lab,
			SAMPLE_IDS.studio,
			SAMPLE_IDS.warehouse,
		]);
		await click(header(/^Check-in/));
		expect(rowIds(container).at(-1)).toBe(SAMPLE_IDS.cold);
	});

	test(
		"200 devices page at 50 and Show 50 more adds a page",
		async () => {
			const { container } = await mountFleet({
				seed: generateFleet(200).input,
				unlock: "none",
			});
			expect(rowIds(container).length).toBe(50);
			expect(
				table(container).parentElement?.parentElement?.textContent,
			).toMatch(/Showing 50 of \d+/);
			await click(byRole("button", "Show 50 more"));
			expect(rowIds(container).length).toBe(100);
		},
		SCALE_TIMEOUT_MS,
	);
});

describe("FleetDevicesTable revoked devices", () => {
	test("sit in a collapsed group that says what is still billed, and open on demand", async () => {
		const { container } = await mountFleet();
		const toggle = container.querySelector(
			"[data-revoked-toggle]",
		) as HTMLElement;
		expect(toggle.textContent).toBe("Revoked · 2");
		expect(toggle.getAttribute("aria-expanded")).toBe("false");
		expect(toggle.closest("tr")?.textContent).toContain(
			"1 still billed to you",
		);
		expect(rowOf(container, SAMPLE_IDS.oldKiosk)).toBeNull();

		await click(toggle);
		expect(toggle.getAttribute("aria-expanded")).toBe("true");
		const kiosk = cells(rowOf(container, SAMPLE_IDS.oldKiosk));
		expect(kiosk[1]).toContain("Revoked");
		expect(kiosk[2]).toContain("Revoked");
		expect(kiosk[3]).toBe("Not availablerevoked devices aren't read");
		expect(kiosk[4]).toBe("–");
		expect(kiosk[5]).toContain("Unusable keys here");
		const partner = cells(rowOf(container, SAMPLE_IDS.partner));
		expect(partner[1]).toContain("Still billed to you");
		expect(partner[5]).toContain("Cloud approvals only");
		expect(partner[5]).toMatch(/€\d+\.\d\d of €\d+\.\d\d/);
	});

	test("the Revoked filter lists them inline; a revocation date shows only when the hub kept one", async () => {
		const seed = sampleFleet();
		const kiosk = seed.devices.find(
			(row) => row.device_id === SAMPLE_IDS.oldKiosk,
		);
		if (kiosk) kiosk.revoked_at = SAMPLE_NOW - 20 * 86_400;
		const { container } = await mountFleet({ seed, search: "filter=revoked" });
		expect(rowIds(container).sort()).toEqual(
			[SAMPLE_IDS.oldKiosk, SAMPLE_IDS.partner].sort(),
		);
		expect(container.querySelector("[data-revoked-toggle]")).toBeNull();
		expect(cells(rowOf(container, SAMPLE_IDS.oldKiosk))[2]).toMatch(
			/^Revokedon Sep 10 · last check-in /,
		);
		expect(cells(rowOf(container, SAMPLE_IDS.partner))[2]).not.toMatch(
			/Revokedon /,
		);
	});

	test("their row actions come from the open items and go where the item goes", async () => {
		const { container, navigations } = await mountFleet({
			search: "filter=revoked",
		});
		const partner = rowOf(container, SAMPLE_IDS.partner);
		await click(byRole("button", /^Revoke spending limit/, partner));
		expect(navigations.at(-1)).toEqual({
			mode: "push",
			href: `/settings/devices?device=${SAMPLE_IDS.partner}&tab=access`,
		});
	});
});
