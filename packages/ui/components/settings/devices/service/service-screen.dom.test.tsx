import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	advance,
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
// Device-lib modules load after the DOM exists (a static import breaks combined runs, W2-FAKES).
const { SAMPLE_IDS } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useActivityTray } = await import("../shell/activity-tray");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { ServiceScreen } = await import("./service-screen");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useActivityTray.getState().setOpen(false);
	useOverlayStore.getState().close();
});
afterAll(dom.restore);

const EDGE = SAMPLE_IDS.edge;
const WAREHOUSE = SAMPLE_IDS.warehouse;
const INVOICE_AI = "app_invoice_ai";

function Page() {
	const { route, scope } = useDevicesRoute();
	if (route.screen !== "service")
		return <p data-left="">{`left:${route.screen}`}</p>;
	return (
		<ServiceScreen
			route={route}
			scope={scope}
			deviceId={route.deviceId}
			serviceId={route.serviceId}
		/>
	);
}

async function open(
	deviceId: string,
	serviceId: string,
	options: MountDevicesOptions & { tab?: string } = {},
) {
	const { tab = "status", ...rest } = options;
	const view = await mountDevices(<Page />, {
		search: `${rest.host === "app" ? `id=${INVOICE_AI}&` : ""}device=${deviceId}&service=${serviceId}&tab=${tab}`,
		...rest,
	});
	await view.settle();
	return view;
}

type View = Awaited<ReturnType<typeof open>>;

const text = (root: ParentNode = document.body) =>
	(root.textContent ?? "").replace(/\s+/g, " ");
const commandTypes = (view: View) =>
	view.fake.api.commands.map(([, type]) => type);
const writes = (view: View) =>
	commandTypes(view).filter((type) =>
		["start", "stop", "restart", "scale", "remove"].includes(type),
	);
const primaries = () => document.querySelectorAll("[data-dv-primary]").length;
const actions = (view: View) =>
	view.container.querySelector<HTMLElement>("[data-service-actions]");

/** R3: wire values and condition keys never reach the screen. */
const MACHINE =
	/crash_looping|update_in_progress|stopped_by_user|failed_stopped|rolled_back|rollout_|placement|replica|\bgrant\b|activating|validating|offline_writes/;

describe("header and verdict", () => {
	test("a service in a safe update: identity, chips, facts, verdict and one-coral rule", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const page = text(view.container);
		expect(byRole("heading", /invoice-extractor/)).toBeTruthy();
		const app = byRole("link", "Invoice AI", view.container);
		expect(app.getAttribute("href")).toContain(
			`/library/config/devices?id=${INVOICE_AI}`,
		);
		expect(app.getAttribute("href")).toContain(`focus=${EDGE}`);
		expect(
			byRole("link", /on edge-berlin-01/, view.container).getAttribute("href"),
		).toContain("tab=services");
		expect(page).toContain("0 of 1 ready · max 1");
		expect(page).toContain("Settings v12");
		expect(page).toContain("running v11");
		expect(page).toContain("Switching over");
		expect(page).toContain("How it runs");
		expect(page).toContain("Runs online · data stays in the cloud");
		expect(page).toContain("Serves");
		expect(page).toContain("Extract invoice");
		expect(page).toContain("Also on edge-berlin-01");
		expect(page).toContain("Updating to settings v12 with a safe update.");
		expect(page).toContain("the device restores v11 on its own");
		expect(primaries()).toBeLessThanOrEqual(1);
		expect(page).not.toMatch(MACHINE);
	});

	test("the service ID is shown whole and Copy puts it on the clipboard", async () => {
		const view = await open(EDGE, "invoice-extractor");
		await click(byRole("button", "Copy service ID", view.container));
		expect(dom.clipboard).toContain("invoice-extractor");
	});

	test("a service that runs as asked says so in one sentence and names what it serves", async () => {
		const view = await open(EDGE, "support-bot");
		const page = text(view.container);
		expect(page).toContain(
			"Runs as you asked: settings v7, 2 of 2 instances ready.",
		);
		expect(page).toContain("Offline copy · data lives only on edge-berlin-01");
		expect(page).toContain("Support chat");
		// Its copy is one app version behind: an older version has no name, so the hash carries the drift.
		expect(page).toContain("App version71c6216b1 behind");
		expect(byRole("link", "How to update", view.container)).toBeTruthy();
		expect(page).not.toMatch(MACHINE);
	});

	test("a service behind the published event versions links to How to update on Configuration", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const how = byRole("link", "How to update", view.container);
		expect(how.getAttribute("href")).toContain("tab=configuration");
		expect(view.container.querySelector("[data-drift=behind]")).not.toBeNull();
	});

	test("in an app's settings: no crumbs, the where-else line and links that stay in the app", async () => {
		const view = await open(EDGE, "invoice-extractor", { host: "app" });
		const page = text(view.container);
		expect(queryByRole("navigation", "Breadcrumb", view.container)).toBeNull();
		const line = view.container.querySelector("[data-service-context]");
		expect(text(line as HTMLElement)).toMatch(
			/Invoice AI (also runs as|runs nowhere else you can see)/,
		);
		const where = byRole("link", "Where it runs", view.container);
		expect(where.getAttribute("href")).toContain(
			`/library/config/devices?id=${INVOICE_AI}`,
		);
		expect(where.getAttribute("href")).toContain(`focus=${EDGE}`);
		expect(page).not.toContain("Also on edge-berlin-01");
		expect(primaries()).toBeLessThanOrEqual(1);
	});
});

describe("diagnosis", () => {
	const status = (view: View) =>
		view.container.querySelector("[data-service-status]") as HTMLElement;

	test("write buffering is reported for a service that buffers, and not claimed for one that doesn't", async () => {
		const copy = await open(EDGE, "nightly-sync");
		expect(text(status(copy))).toContain("Requested Stopped, actual Stopped");
		expect(text(status(copy))).not.toContain("Write buffering");
		await copy.unmount();
		const buffering = await open(EDGE, "invoice-extractor");
		expect(text(status(buffering))).toContain(
			"Write buffering up to date · 0 waiting",
		);
	});
});

describe("app context", () => {
	test("a service of another app says so and links into that app's settings", async () => {
		const view = await open(EDGE, "support-bot", { host: "app" });
		const page = text(view.container);
		expect(page).toContain(
			"support-bot runs Support Portal, not the app whose settings you opened.",
		);
		expect(
			byRole(
				"link",
				"Open it in Support Portal's settings",
				view.container,
			).getAttribute("href"),
		).toContain("id=app_support_portal");
		expect(view.container.querySelector("[data-service-context]")).toBeNull();
	});
});

describe("tabs", () => {
	test("seven tabs with the Write buffering label; choosing one replaces the URL", async () => {
		const view = await open(EDGE, "support-bot");
		expect(allByRole("tab", undefined, view.container).map(text)).toEqual([
			"Status",
			"Activity & logsActivity",
			"Metrics",
			"ConfigurationConfig",
			"Endpoint",
			"Cloud accessCloud",
			"Write bufferingBuffering",
		]);
		expect(
			byRole("tab", "Status", view.container).getAttribute("aria-selected"),
		).toBe("true");
		await click(byRole("tab", /^Metrics/, view.container));
		await view.settle();
		const last = view.navigations.at(-1);
		expect(last?.mode).toBe("replace");
		expect(last?.href).toContain("tab=metrics");
		expect(last?.href).toContain("service=support-bot");
	});

	test("another tab renders that tab's own component instead of the Status blocks", async () => {
		const view = await open(EDGE, "support-bot", { tab: "metrics" });
		expect(
			byRole("tab", /^Metrics/, view.container).getAttribute("aria-selected"),
		).toBe("true");
		expect(view.container.querySelector("[data-service-status]")).toBeNull();
		expect(byRole("tabpanel", undefined, view.container)).toBeTruthy();
		expect(actions(view)).not.toBeNull();
	});

	test("badges: buffered changes that need you and paused queues", async () => {
		const view = await open(SAMPLE_IDS.studio, "field-notes");
		expect(text(byRole("tab", /Write buffering/, view.container))).toContain(
			"queues need you",
		);
		expect(text(byRole("tab", /Cloud access/, view.container))).toContain(
			"Buffered changes paused because cloud access changed",
		);
		expect(text(view.container)).toContain(
			"Runs as you asked, but buffered changes need you.",
		);
		expect(
			byRole("link", "Review buffered writes", view.container).getAttribute(
				"href",
			),
		).toContain("tab=offline");
	});
});

describe("action bar", () => {
	test("while an update runs only Stop is allowed; a click on Restart sends nothing", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const bar = actions(view) as HTMLElement;
		expect(text(bar)).toContain(
			"An update is in progress. Only Stop is allowed.",
		);
		const restart = byRole("button", "Restart…", bar);
		expect(restart.getAttribute("aria-disabled")).toBe("true");
		const reason = bar.querySelector(
			`[id="${restart.getAttribute("aria-describedby")}"]`,
		);
		expect(text(reason as HTMLElement)).toBe(
			"An update is in progress. Only Stop is allowed.",
		);
		const stop = byRole("button", "Stop…", bar);
		expect(stop.getAttribute("aria-disabled")).toBe(null);
		expect(stop.getAttribute("aria-describedby")).toBe(null);
		const hubWrites = view.fake.api.writes().length;
		await click(restart);
		await view.settle();
		expect(writes(view)).toEqual([]);
		expect(view.fake.api.writes()).toHaveLength(hubWrites);
		expect(view.container.querySelector("[data-inline-confirm]")).toBeNull();
	});

	test("max one instance: a sentence with the reason replaces the stepper", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const bar = actions(view) as HTMLElement;
		expect(text(bar)).toContain(
			"This service runs one instance because write buffering is on.",
		);
		expect(queryByRole("button", "One instance more", bar)).toBeNull();
	});

	test("Open service page: a link when this computer knows the address, the Endpoint tab when the service listens on every network", async () => {
		const local = await open(EDGE, "invoice-extractor");
		const page = byRole("link", "Open service page", local.container);
		expect(page.getAttribute("href")).toBe("http://127.0.0.1:8081/ui/");
		expect(page.getAttribute("target")).toBe("_blank");
		await local.unmount();
		const everywhere = await open(EDGE, "support-bot");
		await click(byRole("button", "Open service page…", everywhere.container));
		await everywhere.settle();
		expect(everywhere.navigations.at(-1)?.href).toContain("tab=endpoint");
	});

	test("Stop… asks inline with the consequence rows, then sends one stop and shows the result", async () => {
		const view = await open(EDGE, "support-bot");
		const bar = actions(view) as HTMLElement;
		await click(byRole("button", "Stop…", bar));
		const confirm = view.container.querySelector(
			"[data-inline-confirm]",
		) as HTMLElement;
		expect(text(confirm)).toContain("Stop support-bot?");
		expect(text(confirm)).toContain("The service stops answering");
		expect(writes(view)).toEqual([]);
		// The question takes the focus, so the keyboard and a screen reader are where it is.
		expect(document.activeElement).toBe(
			byRole("button", "Stop support-bot", confirm),
		);
		await click(byRole("button", "Stop support-bot", confirm));
		await view.settle();
		expect(writes(view)).toEqual(["stop"]);
		expect(view.container.querySelector("[data-inline-confirm]")).toBeNull();
		expect(text(view.container)).toMatch(
			/Stop support-bot: (in progress|done)/,
		);
	});

	test("an open confirm stays with its service: the next service's page starts clean", async () => {
		const view = await open(EDGE, "support-bot");
		await click(byRole("button", "Stop…", actions(view) as HTMLElement));
		expect(
			view.container.querySelector("[data-inline-confirm]"),
		).not.toBeNull();
		await click(byRole("link", "nightly-sync", view.container));
		await view.settle();
		expect(byRole("heading", /nightly-sync/)).toBeTruthy();
		expect(view.container.querySelector("[data-inline-confirm]")).toBeNull();
		expect(writes(view)).toEqual([]);
	});

	test("Cancel in the inline confirm sends nothing", async () => {
		const view = await open(EDGE, "support-bot");
		const restart = byRole("button", "Restart…", actions(view) as HTMLElement);
		await click(restart);
		await click(byRole("button", "Cancel", view.container));
		expect(writes(view)).toEqual([]);
		expect(view.container.querySelector("[data-inline-confirm]")).toBeNull();
		expect(document.activeElement).toBe(restart);
	});

	test("a stopped service offers Start, which starts it after the inline confirm", async () => {
		const view = await open(EDGE, "nightly-sync");
		const bar = actions(view) as HTMLElement;
		expect(queryByRole("button", "Stop…", bar)).toBeNull();
		expect(text(view.container)).toContain("Stopped, as you asked.");
		// Its settings weren't read here; the app's events say why it can't run more than one instance.
		expect(text(bar)).toContain(
			"This service runs one instance: its background event runs once per device.",
		);
		await click(byRole("button", "Start", bar));
		await click(byRole("button", "Start nightly-sync", view.container));
		await view.settle();
		expect(writes(view)).toEqual(["start"]);
	});

	test("instances: one more runs at once, one fewer asks first", async () => {
		const view = await open(EDGE, "support-bot");
		const bar = actions(view) as HTMLElement;
		expect(text(bar)).toContain("of max 4 · 2 ready");
		await click(byRole("button", "One instance more", bar));
		await view.settle();
		const scale = () =>
			view.fake.api.commands
				.filter(([, type]) => type === "scale")
				.map(([, , command]) => command.replicas);
		expect(scale()).toEqual([3]);
		await click(
			byRole("button", "One instance fewer", actions(view) as HTMLElement),
		);
		const confirm = view.container.querySelector(
			"[data-inline-confirm]",
		) as HTMLElement;
		expect(confirm).not.toBeNull();
		expect(scale()).toHaveLength(1);
		await click(
			byRole("button", /^Run \d instances? of support-bot$/, confirm),
		);
		await view.settle();
		expect(scale()).toHaveLength(2);
	});

	test("the overflow menu: copy, open, where the app runs, the two wizard entries and Remove", async () => {
		const view = await open(EDGE, "support-bot");
		await click(
			byRole("button", "More actions for support-bot", view.container),
		);
		const menu = inPortal("menu");
		const items = allByRole("menuitem", undefined, menu).map(text);
		expect(items[0]).toBe("Copy service ID");
		expect(items[1]).toBe("Copy deployment ID");
		expect(items).toContain("Open edge-berlin-01");
		expect(
			items.some((item) => item.startsWith("Where Support Portal runs")),
		).toBe(true);
		// The copy is one version behind, so the entry names where the update leads.
		expect(items.some((item) => item.startsWith("Update to v2.4.0…"))).toBe(
			true,
		);
		expect(items.some((item) => item.startsWith("Add an event…"))).toBe(true);
		const remove = byRole("menuitem", /Remove service…/, menu);
		expect(text(remove)).toContain("In Configuration › Danger zone");
		await click(remove);
		await view.settle();
		expect(view.navigations.at(-1)?.href).toContain("tab=configuration");
		expect(writes(view)).toEqual([]);
	});

	test("Update… opens the deploy wizard for this service in the page's scope", async () => {
		const view = await open(EDGE, "support-bot");
		await click(
			byRole("button", "More actions for support-bot", view.container),
		);
		await click(byRole("menuitem", /^Update to v2\.4\.0…/, inPortal("menu")));
		await view.settle();
		const href = view.navigations.at(-1)?.href ?? "";
		expect(href).toContain("flow=deploy");
		expect(href).toContain("service=support-bot");
		expect(href).toContain(`device=${EDGE}`);
	});
});

describe("states", () => {
	test("locked: one coral Unlock…, the reason instead of an empty page, and nothing is sent", async () => {
		const view = await open(EDGE, "invoice-extractor", { unlock: "none" });
		const page = text(view.container);
		expect(page).toContain("Unlock edge-berlin-01 to see this service.");
		expect(actions(view)).toBeNull();
		expect(view.container.querySelector("[data-gate=locked]")).not.toBeNull();
		expect(primaries()).toBe(1);
		const before = view.fake.api.commands.length;
		const hubWrites = view.fake.api.writes().length;
		await click(
			view.container.querySelector("[data-dv-primary]") as HTMLElement,
		);
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "unlock",
			deviceId: EDGE,
		});
		expect(view.fake.api.commands.length).toBe(before);
		expect(view.fake.api.writes()).toHaveLength(hubWrites);
		expect(page).not.toMatch(MACHINE);
	});

	test("offline with a last known crash: banner, last-known verdict, Start gated with the reason", async () => {
		const view = await open(WAREHOUSE, "scanner-ingest");
		const page = text(view.container);
		expect(page).toContain(
			"warehouse-pi is offline. Everything below is this service's last known state.",
		);
		expect(page).toContain("Crashing when last seen");
		expect(page).toContain("Crashing: an instance keeps restarting");
		expect(text(byRole("tab", /^Status/, view.container))).toContain(
			"The service isn't running as requested",
		);
		const start = byRole("button", "Start", actions(view) as HTMLElement);
		expect(start.getAttribute("aria-disabled")).toBe("true");
		expect(text(actions(view) as HTMLElement)).toMatch(/offline/);
		await click(start);
		await view.settle();
		expect(writes(view)).toEqual([]);
		expect(page).toContain("Needs a live connection");
		expect(primaries()).toBeLessThanOrEqual(1);
		expect(page).not.toMatch(MACHINE);
		await click(byRole("button", "Diagnose", view.container));
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "diagnose",
			deviceId: WAREHOUSE,
		});
	});

	test("a service the device doesn't have: says so and links to the device's services", async () => {
		const view = await open(EDGE, "no-such-service");
		expect(text(view.container)).toContain(
			"No service no-such-service on edge-berlin-01",
		);
		expect(actions(view)).toBeNull();
		expect(queryByRole("button", "Unlock…", view.container)).toBeNull();
		expect(primaries()).toBe(0);
		expect(
			byRole(
				"link",
				"Open edge-berlin-01's services",
				view.container,
			).getAttribute("href"),
		).toContain("tab=services");
	});

	test("a device that never checked in: says so, without an unlock that couldn't show anything", async () => {
		const view = await open(SAMPLE_IDS.cold, "any-service");
		const page = text(view.container);
		expect(page).toContain("cold-storage-nas hasn't checked in yet.");
		expect(page).toContain("No status yet");
		expect(primaries()).toBe(0);
		expect(actions(view)).toBeNull();
		expect(page).not.toMatch(MACHINE);
	});

	test("a shared device that is locked: the facts say unknown until unlocked", async () => {
		const view = await open(SAMPLE_IDS.lab, "invoice-extractor-gpu");
		const page = text(view.container);
		expect(page).toContain("Unlock lab-gpu-02 to see this service.");
		expect(page).toContain("Unknown until unlocked");
		expect(primaries()).toBe(1);
	});

	test("unmounting while a read is in flight drops the late answer", async () => {
		const view = await open(EDGE, "invoice-extractor", { viewFacts: false });
		const release = view.fake.agent(EDGE).hold("rollout_history");
		await view.unmount();
		release();
		await advance(20);
		expect(document.querySelector("[role=alert]")).toBeNull();
	});
});

describe("older hub and older agent", () => {
	test("older agent: latest-only history and on-device errors, without any newer command", async () => {
		const view = await open(EDGE, "invoice-extractor", { agentFeatures: {} });
		const page = text(view.container);
		expect(page).toContain("only on the device");
		expect(page).toMatch(
			/Only the latest finished update is shown|No finished update is known on this computer/,
		);
		expect(page).toContain("flow-like-standalone status");
		expect(
			view.container.querySelector("[data-tone=critical][role=alert]"),
		).toBeNull();
		const sent = commandTypes(view);
		for (const newer of [
			"rollout_history",
			"operations",
			"host_operation",
			"metrics_history",
		])
			expect(sent).not.toContain(newer);
		expect(page).not.toMatch(MACHINE);
	});

	test("older hub: the page renders without an error and asks each new route once", async () => {
		const view = await open(EDGE, "invoice-extractor", { hubVersion: "old" });
		const page = text(view.container);
		expect(page).toContain("Updating to settings v12 with a safe update.");
		expect(view.container.querySelector("[role=alert]")).toBeNull();
		expect(
			view.fake.api.sent("GET", /device-placements$/).length,
		).toBeLessThanOrEqual(1);
		expect(
			view.fake.api.sent("GET", new RegExp(`${EDGE}/management/my-access$`))
				.length,
		).toBeLessThanOrEqual(1);
		expect(primaries()).toBeLessThanOrEqual(1);
	});
});
