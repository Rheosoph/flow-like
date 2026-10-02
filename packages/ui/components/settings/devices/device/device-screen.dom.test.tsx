import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import {
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";

const dom = installDom();
const kit = await import("./device-test-kit");
const { act } = await import("react");
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { fakeKeys } = await import("../testing/fake-device-api");
const { sampleFleet } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);

const {
	IDS,
	APPS,
	MACHINE,
	lastNavigation,
	openDevice,
	primaries,
	text,
	useOverlayStore,
} = kit;

afterEach(async () => {
	await kit.resetDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const tabNames = (root: ParentNode) =>
	allByRole("tab", undefined, root).map((tab) =>
		(tab.textContent ?? "").replace(/\d.*$/, "").trim(),
	);

describe("header", () => {
	test("an unlocked, connected device: identity, chips, facts and one coral action", async () => {
		const view = await openDevice(IDS.edge);
		const page = text(view.container);
		expect(byRole("heading", /edge-berlin-01/, view.container)).toBeTruthy();
		expect(
			view.container.querySelector("[data-relationship=owner]")?.textContent,
		).toBe("Yours");
		expect(page).toContain("Online · checked in");
		expect(
			view.container.querySelector("[data-health=attention]")?.textContent,
		).toBe("Needs attention · 3");
		expect(page).toContain("Live ·");
		expect(page).toContain("Backed up (v3)");
		expect(page).toContain("Locks after 30 min unused");
		expect(page).toContain("PlatformLinux · sandbox required");
		expect(page).toContain("Agent0.9.4· newest verified");
		expect(page).toContain("Last restart");
		const deploy = byRole("link", "Deploy an app…", view.container);
		expect(deploy.getAttribute("href")).toContain("flow=deploy");
		expect(deploy.getAttribute("href")).toContain(`device=${IDS.edge}`);
		expect(deploy.hasAttribute("data-dv-primary")).toBe(true);
		expect(byRole("button", "Lock", view.container)).toBeTruthy();
		expect(primaries()).toBe(1);
		expect(page).not.toMatch(MACHINE);
	});

	test("the device ID is copied whole", async () => {
		const view = await openDevice(IDS.edge);
		await click(allByRole("button", "Copy device ID", view.container)[0]);
		expect(dom.clipboard).toContain(IDS.edge);
	});

	test("the crumb leads back to the fleet", async () => {
		const view = await openDevice(IDS.edge);
		const crumb = byRole("link", "Devices", view.container);
		expect(crumb.getAttribute("href")).toBe("/settings/devices");
		await click(crumb);
		expect(text(view.container)).toContain("left:fleet");
	});

	test("the overflow menu: copy, disconnect, diagnose, lock and revoke", async () => {
		const view = await openDevice(IDS.edge);
		await click(
			byRole("button", "More actions for edge-berlin-01", view.container),
		);
		const menu = inPortal("menu");
		const items = allByRole("menuitem", undefined, menu).map((item) =>
			(item.textContent ?? "").trim(),
		);
		expect(items).toEqual([
			"Copy device ID",
			"Disconnect",
			"Diagnose connection…",
			"Lock",
			"Revoke device…",
		]);
		await click(byRole("menuitem", "Diagnose connection…", menu));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "diagnose",
			deviceId: IDS.edge,
		});
	});

	test("Revoke device… in the menu jumps to the danger zone and opens the flow", async () => {
		const view = await openDevice(IDS.edge);
		const writes = view.fake.api.writes().length;
		await click(
			byRole("button", "More actions for edge-berlin-01", view.container),
		);
		await click(byRole("menuitem", "Revoke device…", inPortal("menu")));
		await view.settle();
		expect(
			view.navigations.some((entry) => entry.href.includes("tab=settings")),
		).toBe(true);
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain("Revoke edge-berlin-01?");
		expect(view.fake.api.writes().length).toBe(writes);
	});

	test("Lock closes the keys: Unlock… becomes the one coral action and the last data stays, greyed", async () => {
		const view = await openDevice(IDS.edge, { tab: "services" });
		await click(byRole("button", "Lock", view.container));
		await view.settle();
		const page = text(view.container);
		expect(
			byRole("button", "Unlock…", view.container).hasAttribute(
				"data-dv-primary",
			),
		).toBe(true);
		expect(primaries()).toBe(1);
		expect(page).toContain("Locked. Showing what was read at");
		expect(page).toContain("2 of 3 services were as you asked when last read.");
		expect(page).toContain("Last known ·");
		expect(page).toContain("Unlock edge-berlin-01 to run commands.");
		expect(
			view.container.querySelectorAll("tr[data-service][data-dim]").length,
		).toBe(3);
	});
});

describe("verdict", () => {
	test("names how many services are as asked, the one switching over and the most urgent item", async () => {
		const view = await openDevice(IDS.edge);
		const verdict = view.container.querySelector("[data-headline]");
		expect(text(verdict as HTMLElement)).toBe(
			"2 of 3 services are as you asked.invoice-extractor is switching to new settings; it started 4 minutes ago. 3 items need you; the most urgent: certificate expiring.",
		);
	});

	test("offline: since when, and what it last reported", async () => {
		const view = await openDevice(IDS.warehouse);
		const verdict = text(
			view.container.querySelector("[data-headline]") as HTMLElement,
		);
		expect(verdict).toContain("warehouse-pi has been offline since");
		expect(verdict).toContain("(3 hours ago)");
		expect(verdict).toMatch(
			/When it last reported \(.+\), scanner-ingest kept crashing\. A certificate had expired too\./,
		);
	});

	test("never checked in: registered when, and that its keys exist only here", async () => {
		const view = await openDevice(IDS.cold);
		expect(
			text(view.container.querySelector("[data-headline]") as HTMLElement),
		).toBe(
			"Registered 12 minutes ago and hasn't checked in yet.Its keys exist only on this computer.",
		);
	});
});

describe("tabs", () => {
	test("eight sections with the open items of each as a badge", async () => {
		const view = await openDevice(IDS.edge);
		const tabs = allByRole("tab", undefined, view.container);
		expect(tabs).toHaveLength(8);
		const badge = (name: RegExp) =>
			byRole("tab", name, view.container).querySelector("[data-tab-count]")
				?.textContent ?? "";
		expect(text(byRole("tab", /^Overview/, view.container))).toContain("3");
		expect(text(byRole("tab", /^Certificates/, view.container))).toContain(
			"1 item needs you",
		);
		expect(text(byRole("tab", /^Access/, view.container))).toContain(
			"1 item needs you",
		);
		expect(badge(/^Keys/)).toBe("");
		expect(
			byRole("tab", /^Overview/, view.container).getAttribute("aria-selected"),
		).toBe("true");
	});

	test("choosing a tab replaces the URL and shows that section", async () => {
		const view = await openDevice(IDS.edge);
		await click(byRole("tab", /^Services/, view.container));
		await view.settle();
		expect(lastNavigation(view)?.[0]).toBe("replace");
		expect(lastNavigation(view)?.[1]).toContain("tab=services");
		expect(
			byRole("table", "Services on edge-berlin-01", view.container),
		).toBeTruthy();
	});

	test("the tabs of the other lanes render in their panels", async () => {
		const view = await openDevice(IDS.edge);
		for (const name of [
			/^Activity/,
			/^Metrics/,
			/^Certificates/,
			/^Access/,
			/^Keys/,
		]) {
			const tab = byRole("tab", name, view.container);
			await click(tab);
			await view.settle();
			expect(tab.getAttribute("aria-selected")).toBe("true");
			const panel = view.container.querySelector("[role=tabpanel]");
			expect((panel?.textContent ?? "").length).toBeGreaterThan(0);
		}
	});

	test("a revoked device keeps Overview, Access and Keys, and says why the rest is gone", async () => {
		const view = await openDevice(IDS.oldKiosk, { tab: "services" });
		expect(tabNames(view.container)).toEqual(["Overview", "Access", "Keys"]);
		const page = text(view.container);
		expect(page).toContain(
			"Services, activity, metrics, certificates and device settings aren't shown: revoked devices aren't read.",
		);
		expect(
			byRole("tab", /^Overview/, view.container).getAttribute("aria-selected"),
		).toBe("true");
		expect(page).toContain("old-kiosk is revoked.");
		expect(page).toContain("Revoked.");
		expect(primaries()).toBe(0);
		await click(
			byRole("button", "Delete keys from this computer…", view.container),
		);
		expect(lastNavigation(view)?.[1]).toContain("tab=keys");
	});

	test("someone who only pays for cloud access sees Overview and Access", async () => {
		const view = await openDevice(IDS.partner);
		expect(tabNames(view.container)).toEqual(["Overview", "Access"]);
		const page = text(view.container);
		expect(page).toContain(
			"Only cloud approvals are shown: you have no access to this device itself.",
		);
		expect(page).toContain("Revoked by its owner.");
		expect(page).toContain(
			"You still pay for its spending limit: €12.50 of €50.00 used.",
		);
		expect(view.container.querySelector("[data-key]")).toBeNull();
		expect(page).not.toContain("Platform");
		expect(page).not.toMatch(MACHINE);
	});
});

describe("states", () => {
	test("locked: one coral Unlock…, the hub's view of the device, and nothing is sent", async () => {
		const view = await openDevice(IDS.edge, { unlock: "none" });
		const page = text(view.container);
		expect(page).toContain("1 item needs you, as far as the hub knows.");
		expect(page).toContain("Unlock to see your services.");
		expect(page).toContain("PlatformUnknown until unlocked");
		expect(page).toContain("Unlock edge-berlin-01 to see its services.");
		expect(primaries()).toBe(1);
		const commands = view.fake.api.commands.length;
		const writes = view.fake.api.writes().length;
		await click(byRole("button", "Unlock…", view.container));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: IDS.edge,
			connectLive: true,
		});
		expect(view.fake.api.commands.length).toBe(commands);
		expect(view.fake.api.writes().length).toBe(writes);
		expect(page).not.toMatch(MACHINE);
	});

	test("no keys on this computer: Restore keys… leads to Keys & recovery", async () => {
		const seed = sampleFleet();
		seed.local.vaults = seed.local.vaults.filter(
			(vault) => vault.deviceId !== IDS.edge,
		);
		seed.keys = seed.keys.filter((session) => session.deviceId !== IDS.edge);
		delete seed.local.backups[IDS.edge];
		const view = await openDevice(IDS.edge, { seed, tab: "services" });
		const page = text(view.container);
		const restore = allByRole("link", "Restore keys…", view.container);
		expect(restore.length).toBeGreaterThanOrEqual(1);
		expect(restore[0].getAttribute("href")).toContain("view=keys");
		expect(restore[0].getAttribute("href")).toContain(`focus=${IDS.edge}`);
		expect(page).toContain("This computer has no keys for edge-berlin-01.");
		expect(page).toContain(
			"This computer has no keys for it, so its services can't be read here.",
		);
		expect(primaries()).toBe(0);
		expect(view.container.querySelector("[data-kind=empty]")).toBeNull();
	});

	test("a changed identity blocks management with a critical banner; revoking still works", async () => {
		const fake = await createFakeWorkspace(undefined, { unlock: "none" });
		const row = fake.hub.rows.get(IDS.edge);
		if (!row) throw new Error("the sample has no edge device");
		row.identity = fakeKeys.identity("someone-else");
		const view = await openDevice(IDS.edge, { fake, tab: "settings" });
		await view.settle();
		const banner = byRole("alert", undefined, view.container);
		expect(text(banner)).toContain(
			"The hub reports different keys for edge-berlin-01 than the ones you trusted on",
		);
		expect(text(banner)).toContain(
			"Management is blocked: no commands, deploys or access changes until you confirm the identity. Revoking still works, because it happens at the hub.",
		);
		const page = text(view.container);
		expect(page).toContain(
			"Management is blocked until the identity is confirmed.",
		);
		expect(queryByRole("button", "Unlock…", view.container)).toBeNull();
		expect(primaries()).toBe(0);
		const reboot = byRole("button", "Reboot device…", view.container);
		expect(reboot.getAttribute("aria-disabled")).toBe("true");
		await click(reboot);
		expect(view.fake.api.commands).toEqual([]);
		const revoke = byRole("button", "Revoke edge-berlin-01…", view.container);
		expect(revoke.getAttribute("aria-disabled")).toBeNull();
		await click(byRole("button", "Review identity…", banner));
		expect(useOverlayStore.getState().overlay.kind).toBe("unlock");
	});

	test("offline: the banner says since when, and Deploy is disabled with the reason", async () => {
		const view = await openDevice(IDS.warehouse);
		const page = text(view.container);
		expect(page).toContain(
			"warehouse-pi is offline. Everything below is its last known state.",
		);
		expect(page).toContain(
			"Actions that need a live connection are disabled until it checks in again.",
		);
		const deploy = byRole("button", "Deploy an app…", view.container);
		expect(deploy.getAttribute("aria-disabled")).toBe("true");
		const before = view.navigations.length;
		const writes = view.fake.api.writes().length;
		await click(deploy);
		expect(view.navigations.length).toBe(before);
		expect(view.fake.api.writes().length).toBe(writes);
		expect(primaries()).toBe(0);
	});

	test("a person the device is shared with reads what they may do, where and until when", async () => {
		const view = await openDevice(IDS.lab);
		const access = view.container.querySelector(
			"[data-device-access]",
		) as HTMLElement;
		expect(text(access)).toMatch(
			/^Your accessView status, Read metrics, Read logs, Deploy & configure, Start services, and Stop services · App Invoice AI · until /,
		);
		expect(byRole("link", "Invoice AI", access).getAttribute("href")).toContain(
			`/library/config/devices?id=${APPS.invoiceAi}`,
		);
		const page = text(view.container);
		expect(page).toContain("Healthy as far as the hub knows.");
		expect(page).toContain("Unlock to see your services in Invoice AI.");
		expect(
			view.container.querySelector("[data-relationship=shared]"),
		).not.toBeNull();
	});

	test("an unknown device id says so and leads back to the list", async () => {
		const view = await openDevice("00000000-0000-4000-8000-000000000000");
		expect(text(view.container)).toContain("This device isn't in your list");
		await click(byRole("button", "Back to devices", view.container));
		expect(text(view.container)).toContain("left:fleet");
	});

	test("a hub that stops answering keeps the page and stamps its data with since when", async () => {
		const view = await openDevice(IDS.edge);
		view.fake.api.fail(
			{ method: "GET", path: "devices" },
			new ApiResponseError({ status: 403, code: "FORBIDDEN", message: "no" }),
		);
		await act(async () => {
			await view.fake.queryClient.refetchQueries();
		});
		await view.settle();
		const page = text(view.container);
		expect(byRole("heading", /edge-berlin-01/, view.container)).toBeTruthy();
		expect(page).toContain("couldn't refresh");
		expect(page).toContain("2 of 3 services are as you asked.");
		expect(
			view.container.querySelector(
				'[data-stamp][data-src="hub"][data-age="error"]',
			),
		).not.toBeNull();
	});
});

describe("in an app's settings", () => {
	test("the header deploys this app, the verdict answers for it, and other tabs are marked whole-device", async () => {
		const view = await openDevice(IDS.edge, {
			app: APPS.invoiceAi,
			tab: "services",
		});
		const page = text(view.container);
		const deploy = allByRole("link", "Deploy Invoice AI here…", view.container);
		expect(deploy[0].hasAttribute("data-dv-primary")).toBe(true);
		expect(deploy[0].getAttribute("href")).toContain("flow=deploy");
		expect(deploy[0].getAttribute("href")).toContain("mode=new");
		expect(primaries()).toBe(1);
		expect(page).toContain("Invoice AI is updating here.");
		expect(page).toContain("Elsewhere on edge-berlin-01, 3 items need you.");
		expect(page).toContain(
			"Services, and the items and services on Overview, show only this app. The rest, including tabs marked with the device glyph, covers the whole device.",
		);
		expect(text(byRole("tab", /^Certificates/, view.container))).toContain(
			", whole device",
		);
		expect(text(byRole("tab", /^Services/, view.container))).not.toContain(
			", whole device",
		);
		const crumb = byRole("link", "Devices", view.container);
		expect(crumb.getAttribute("href")).toContain(`id=${APPS.invoiceAi}`);
		expect(crumb.getAttribute("href")).toContain(`focus=${IDS.edge}`);
	});

	test("an app that doesn't run here says what a service would be", async () => {
		const view = await openDevice(IDS.studio, {
			app: APPS.invoiceAi,
			tab: "overview",
		});
		const page = text(view.container);
		expect(page).toContain("Invoice AI doesn't run on studio-mac-mini yet.");
		expect(page).toContain(
			"It's an online app, so a service here would run it online with its data in the cloud.",
		);
		expect(page).toMatch(/\d of its \d events can run on a device\./);
		expect(page).toContain("Invoice AI on this device");
		expect(page).toContain("Invoice AI doesn't run on studio-mac-mini");
	});
});

describe("older hub and older agent", () => {
	test("older hub: no error, the access line asks to unlock, and each new route is asked once", async () => {
		const view = await openDevice(IDS.lab, { hubVersion: "old" });
		const page = text(view.container);
		expect(page).toContain("Your accessUnlock to check your permissions.");
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
		expect(view.container.querySelector("[data-kind=error]")).toBeNull();
		expect(
			view.fake.api.sent("GET", new RegExp(`${IDS.lab}/management/my-access$`))
				.length,
		).toBeLessThanOrEqual(1);
		expect(
			view.fake.api.sent("GET", "devices/usage").length,
		).toBeLessThanOrEqual(1);
		expect(primaries()).toBeLessThanOrEqual(1);
		expect(page).not.toMatch(MACHINE);
	});

	test("older agent: the page reads without the newer facts and sends no newer command", async () => {
		const view = await openDevice(IDS.edge, { agentFeatures: {} });
		const page = text(view.container);
		expect(byRole("heading", /edge-berlin-01/, view.container)).toBeTruthy();
		expect(page).toContain("services are as you asked");
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
		const sent = kit.commandTypes(view);
		for (const newer of kit.NEWER_COMMANDS.filter(
			(type) => type !== "artifact",
		))
			expect(sent).not.toContain(newer);
		expect(
			view.fake.api.commands.filter(
				([, type, command]) =>
					type === "artifact" &&
					(command.request as { kind?: string } | undefined)?.kind === "usage",
			),
		).toEqual([]);
		expect(page).not.toMatch(MACHINE);
	});
});
