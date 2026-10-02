import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { ServicesUnavailable } from "../../../../lib/device-management/model/device-view";
import {
	allByRole,
	byRole,
	click,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { DeviceView, OpenOptions } from "./device-test-kit";

const dom = installDom();
const kit = await import("./device-test-kit");
const { act } = await import("react");
const { mountDevices } = await import("../testing/mount-devices");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { DeviceDataState } = await import("./services-tab");
const { useDevicePage } = await import("./use-device-page");
const { sampleFleet } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);

const {
	IDS,
	APPS,
	MACHINE,
	commandTypes,
	lastNavigation,
	openDevice,
	primaries,
	text,
} = kit;

afterEach(async () => {
	await kit.resetDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const open = (deviceId: string, options: OpenOptions = {}) =>
	openDevice(deviceId, { tab: "services", ...options });

const row = (view: DeviceView, serviceId: string) => {
	const found = view.container.querySelector<HTMLElement>(
		`tr[data-service="${serviceId}"]`,
	);
	if (!found) throw new Error(`No row for ${serviceId}`);
	return found;
};

const lifecycle = (view: DeviceView) =>
	commandTypes(view).filter((type) =>
		["start", "stop", "restart", "scale", "remove"].includes(type),
	);

describe("services table", () => {
	test("each row: app and how it runs, what it serves, badges, instances, versions and update", async () => {
		const view = await open(IDS.edge);
		const table = byRole("table", "Services on edge-berlin-01", view.container);
		expect(table.querySelectorAll("tr[data-service]").length).toBe(3);
		const invoice = text(row(view, "invoice-extractor"));
		expect(invoice).toContain("Invoice AI· Runs online");
		expect(invoice).toContain("Serves Extract invoice");
		expect(invoice).toContain("Cloud access · €7.41 of €25.00");
		expect(invoice).toContain("Write buffering · up to date");
		expect(invoice).toContain("127.0.0.1:8081");
		expect(invoice).toContain("0 of 1 ready");
		expect(invoice).toContain("App version unknown");
		expect(invoice).toContain("Settings v12");
		expect(invoice).toContain("running v11 · applying");
		expect(invoice).toContain("Switching over");
		expect(invoice).toContain("safe update · v11 → v12");
		const support = text(row(view, "support-bot"));
		expect(support).toContain("Support Portal· Offline copy");
		expect(support).toContain("Serves Support chat · Support API");
		expect(support).toContain("2 of 2 ready");
		expect(support).toContain("No update running");
		expect(text(view.container)).toContain(
			"Requested is what you asked for; actual is what the device reports.",
		);
		expect(
			view.container.querySelector("#services-block [data-stamp]"),
		).not.toBeNull();
		expect(primaries()).toBe(1);
		expect(text(view.container)).not.toMatch(MACHINE);
	});

	test("the actions follow the requested state; an update in progress leaves only Stop", async () => {
		const view = await open(IDS.edge);
		const names = (serviceId: string) =>
			allByRole("button", undefined, row(view, serviceId))
				.map((button) => text(button))
				.filter((name) => /Start|Stop|Restart/.test(name));
		expect(names("support-bot")).toEqual(["Restart…", "Stop…"]);
		expect(names("nightly-sync")).toEqual(["Start"]);
		expect(names("invoice-extractor")).toEqual(["Restart…", "Stop…"]);
		const updating = row(view, "invoice-extractor");
		expect(
			byRole("button", "Restart…", updating).getAttribute("aria-disabled"),
		).toBe("true");
		expect(
			byRole("button", "Stop…", updating).getAttribute("aria-disabled"),
		).toBeNull();
		expect(text(updating)).toContain(
			"An update is in progress. Only Stop is allowed.",
		);
		await click(byRole("button", "Restart…", updating));
		expect(lifecycle(view)).toEqual([]);
	});

	test("Stop… shows what happens in the row, then sends one command and leaves a result", async () => {
		const view = await open(IDS.edge);
		await click(byRole("button", "Stop…", row(view, "support-bot")));
		const confirm = view.container.querySelector<HTMLElement>(
			"[data-inline-confirm]",
		);
		if (!confirm) throw new Error("no inline confirm");
		expect(text(confirm)).toContain("Stop support-bot?");
		expect(confirm.querySelector("[data-conseq]")).not.toBeNull();
		expect(lifecycle(view)).toEqual([]);
		await click(byRole("button", "Stop support-bot", confirm));
		await view.settle();
		expect(lifecycle(view)).toEqual(["stop"]);
		const sent = view.fake.api.commands.find(([, type]) => type === "stop");
		expect(sent?.[0]).toBe(IDS.edge);
		expect(sent?.[2].placement_id).toBe("support-bot");
		expect(view.container.querySelector("[data-inline-confirm]")).toBeNull();
		expect(
			row(view, "support-bot").querySelector("[data-result]"),
		).not.toBeNull();
		expect(
			view.fake.workspace.activity
				.list()
				.some((item) => item.target.serviceId === "support-bot"),
		).toBe(true);
	});

	test("Cancel in the confirm sends nothing", async () => {
		const view = await open(IDS.edge);
		await click(byRole("button", "Restart…", row(view, "support-bot")));
		const confirm = view.container.querySelector<HTMLElement>(
			"[data-inline-confirm]",
		);
		if (!confirm) throw new Error("no inline confirm");
		await click(byRole("button", "Cancel", confirm));
		expect(view.container.querySelector("[data-inline-confirm]")).toBeNull();
		expect(lifecycle(view)).toEqual([]);
	});

	test("the service name and Open lead to the service page without a reload", async () => {
		const view = await open(IDS.edge);
		const name = byRole("link", "support-bot", row(view, "support-bot"));
		expect(name.getAttribute("href")).toContain("service=support-bot");
		await click(name);
		expect(text(view.container)).toContain("left:service");
		expect(lastNavigation(view)).toEqual([
			"push",
			expect.stringContaining("service=support-bot"),
		]);
	});

	test("the app name leads to where that app runs, with this device focused", async () => {
		const view = await open(IDS.edge);
		const app = byRole("link", "Invoice AI", row(view, "invoice-extractor"));
		expect(app.getAttribute("href")).toContain(
			`/library/config/devices?id=${APPS.invoiceAi}`,
		);
		expect(app.getAttribute("href")).toContain(`focus=${IDS.edge}`);
	});

	test("a paused upload names its progress and resumes in the deploy wizard", async () => {
		const view = await open(IDS.edge);
		const nightly = row(view, "nightly-sync");
		expect(text(nightly)).toContain(
			"Uploading a new version · paused at 12 of 38 files",
		);
		const resume = byRole("link", "Resume upload…", nightly);
		expect(resume.getAttribute("href")).toContain("flow=deploy");
		expect(resume.getAttribute("href")).toContain("service=nightly-sync");
		expect(resume.getAttribute("href")).toContain("step=copy_upload");
	});

	test("Deploy an app… in the head opens the wizard for this device; Refresh status re-reads once", async () => {
		const view = await open(IDS.edge);
		const head = view.container.querySelector("#services-block") as HTMLElement;
		const deploy = byRole("link", "Deploy an app…", head);
		expect(deploy.getAttribute("href")).toContain("flow=deploy");
		expect(deploy.hasAttribute("data-dv-primary")).toBe(false);
		const reads = () =>
			commandTypes(view).filter((type) => type === "inspect_page").length;
		const before = reads();
		await click(byRole("button", "Refresh status", head));
		await view.settle();
		expect(reads()).toBeGreaterThan(before);
	});
});

describe("states", () => {
	test("offline: the last known rows stay, dimmed, and commands are disabled with the reason", async () => {
		const view = await open(IDS.warehouse);
		const scanner = row(view, "scanner-ingest");
		expect(scanner.hasAttribute("data-dim")).toBe(true);
		expect(text(scanner)).toContain("last known");
		expect(text(scanner)).toContain(
			"Events unknown: the status snapshot has no event list",
		);
		const start = byRole("button", "Start", scanner);
		expect(start.getAttribute("aria-disabled")).toBe("true");
		expect(text(scanner)).toContain("needs a live connection");
		await click(start);
		expect(view.container.querySelector("[data-inline-confirm]")).toBeNull();
		expect(lifecycle(view)).toEqual([]);
	});

	test("a failed status poll keeps the rows with an error stamp and doesn't ask for the password", async () => {
		const view = await open(IDS.warehouse);
		view.fake.api.fail({ method: "GET", path: /fleet\/snapshots/ });
		await act(async () => {
			await view.fake.workspace.fleet
				.refresh(IDS.warehouse)
				.catch(() => undefined);
		});
		await view.settle();
		expect(row(view, "scanner-ingest")).toBeTruthy();
		const stamp = view.container.querySelector(
			'#services-block [data-stamp][data-src="snap"][data-age="error"]',
		);
		expect(stamp?.textContent).toContain("couldn't refresh · data from");
		expect(queryByRole("dialog")).toBeNull();
		expect(queryByRole("button", "Unlock…", view.container)).toBeNull();
		expect(byRole("button", "Lock", view.container)).toBeTruthy();
		expect(view.container.querySelector("[data-gate=locked]")).toBeNull();
	});

	test("no services yet: an empty state that offers to deploy", async () => {
		const view = await open(IDS.edge);
		view.fake.agent(IDS.edge).placements = [];
		await click(byRole("button", "Refresh status", view.container));
		await view.settle();
		const empty =
			view.container.querySelector<HTMLElement>("[data-kind=empty]");
		if (!empty) throw new Error("no empty state");
		expect(text(empty)).toContain("No services on this device yet");
		expect(byRole("link", "Deploy an app…", empty)).toBeTruthy();
	});

	test("locked with nothing read before: the lock, the viewer's own access, and no command", async () => {
		const view = await open(IDS.lab);
		const gate = view.container.querySelector<HTMLElement>(
			"#services-block [data-gate=locked]",
		);
		if (!gate) throw new Error("no locked gate");
		expect(text(gate)).toContain("Unlock lab-gpu-02 to see its services.");
		expect(text(gate)).toContain(
			"Your access: View status, Read metrics, Read logs, Deploy & configure, Start services, and Stop services · App Invoice AI",
		);
		expect(text(view.container)).toContain(
			"You can see services in Invoice AI only. Other services on lab-gpu-02 aren't shared with you.",
		);
		const refresh = byRole("button", "Refresh status", view.container);
		expect(refresh.getAttribute("aria-disabled")).toBe("true");
		const before = view.fake.api.commands.length;
		await click(refresh);
		expect(view.fake.api.commands.length).toBe(before);
		expect(view.container.querySelector("[data-kind=empty]")).toBeNull();
		expect(primaries()).toBe(1);
	});

	test("never checked in: no status yet, with Diagnose", async () => {
		const view = await open(IDS.cold);
		const state =
			view.container.querySelector<HTMLElement>("[data-kind=never]");
		if (!state) throw new Error("no never state");
		expect(text(state)).toContain("No status yet");
		expect(byRole("button", "Diagnose", state)).toBeTruthy();
	});

	test("while a longer read runs, the block says which part arrived and keeps the rows", async () => {
		const view = await open(IDS.edge);
		const agent = view.fake.agent(IDS.edge);
		const rows = agent.rows();
		let release = () => {};
		const gate = new Promise<void>((resolve) => {
			release = resolve;
		});
		agent.handle("inspect_page", async (command) => {
			if (command.after == null)
				return {
					state: "completed",
					result: {
						device_id: IDS.edge,
						boot_id: agent.bootId,
						placements: rows.slice(0, 2),
						next: rows[1].id,
						...agent.pageFacts(),
					},
				};
			await gate;
			return {
				state: "completed",
				result: {
					device_id: IDS.edge,
					boot_id: agent.bootId,
					placements: rows.slice(2),
					next: null,
				},
			};
		});
		await click(byRole("button", "Refresh status", view.container));
		await view.settle();
		const paging = view.container.querySelector<HTMLElement>(
			"[data-services-paging]",
		);
		if (!paging) throw new Error("no paging line");
		expect(text(paging)).toContain(
			"Reading the service list from the device… part 1 received.",
		);
		expect(view.container.querySelectorAll("tr[data-service]").length).toBe(3);
		release();
		await view.settle();
		expect(view.container.querySelector("[data-services-paging]")).toBeNull();
	});
});

describe("why the services can't be read", () => {
	function State({
		unavailable,
	}: Readonly<{ unavailable: ServicesUnavailable }>) {
		const { scope } = useDevicesRoute();
		const read = useDevicePage(IDS.lab, scope);
		if (read.state !== "ready") return null;
		return (
			<DeviceDataState
				page={{ ...read.page, lockedRows: null, unavailable }}
				what="services"
			/>
		);
	}

	const render = async (unavailable: ServicesUnavailable) => {
		const view = await mountDevices(<State unavailable={unavailable} />, {
			search: `device=${IDS.lab}`,
		});
		await view.settle();
		return view;
	};

	test("no View status: says which permission is missing", async () => {
		const view = await render({
			state: "noaccess",
			reason: { code: "needs_capability" },
		});
		expect(view.container.querySelector("[data-gate=noaccess]")).not.toBeNull();
		expect(text(view.container)).toContain(
			"Your access doesn't include View status.",
		);
	});

	test("access that ended: says so and offers to ask again", async () => {
		const view = await render({
			state: "noaccess",
			reason: { code: "access_ended" },
		});
		expect(text(view.container)).toContain(
			"Your access to lab-gpu-02 has ended.",
		);
		expect(
			byRole("link", "Request access", view.container).getAttribute("href"),
		).toContain("view=access");
	});

	test("a failed read is an error with Try again, not an empty list", async () => {
		const view = await render({ state: "error" });
		expect(view.container.querySelector("[data-kind=error]")).not.toBeNull();
		expect(byRole("button", "Try again", view.container)).toBeTruthy();
		expect(view.container.querySelector("[data-kind=empty]")).toBeNull();
	});

	test("an agent that can't share its status reads as not supported", async () => {
		const view = await render({ state: "unsupported" });
		expect(
			view.container.querySelector("[data-kind=unsupported]"),
		).not.toBeNull();
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
	});
});

describe("in an app's settings", () => {
	test("the app block: how it runs here, how to deploy, and the app's events on this device", async () => {
		const view = await open(IDS.edge, { app: APPS.invoiceAi });
		const app = view.container.querySelector<HTMLElement>("#dv-app");
		if (!app) throw new Error("no app block");
		const page = text(app);
		expect(page).toContain("Invoice AI on edge-berlin-01");
		expect(page).toContain("1 service");
		expect(app.querySelector("[data-mode=online]")).not.toBeNull();
		expect(page).toContain(
			"Invoice AI is an online app, so edge-berlin-01 runs it online at the version you deployed from the hub.",
		);
		expect(page).toContain(
			"Deploy Invoice AI here… puts the events you pick into one service; Deploy here on an event starts with only that event.",
		);
		expect(page).toContain(
			"A new service needs cloud access, approved by an Admin or Owner of Invoice AI.",
		);
		const served = app.querySelector<HTMLElement>("[data-matrix-cell=served]");
		expect(text(served as HTMLElement)).toContain("invoice-extractor");
		const deployHere = allByRole("link", "Deploy here", app);
		expect(deployHere.length).toBeGreaterThanOrEqual(1);
		expect(deployHere[0].getAttribute("href")).toContain("flow=deploy");
		expect(deployHere[0].getAttribute("href")).toContain("mode=new");
		expect(deployHere[0].getAttribute("href")).toContain("event=");
		expect(deployHere[0].getAttribute("href")).toContain(`device=${IDS.edge}`);
		expect(page).toContain(
			"Publishing on the hub doesn't change a running device.",
		);
		expect(
			byRole("link", "Where else Invoice AI runs", app).getAttribute("href"),
		).toContain(`focus=${IDS.edge}`);
		expect(text(view.container)).not.toMatch(MACHINE);
	});

	test("only the app's services are listed, and the others are counted", async () => {
		const view = await open(IDS.edge, { app: APPS.invoiceAi });
		const table = byRole("table", "Services on edge-berlin-01", view.container);
		const names = Array.from(table.querySelectorAll("tr[data-service]"), (el) =>
			el.getAttribute("data-service"),
		);
		expect(names).toEqual(["invoice-extractor"]);
		expect(text(view.container)).toContain("Invoice AI services");
		expect(text(view.container)).toContain(
			"2 services of other apps on edge-berlin-01 aren't shown.",
		);
		expect(primaries()).toBe(1);
	});

	test("events that can't run on devices sit in a collapsed group", async () => {
		const view = await open(IDS.edge, { app: APPS.invoiceAi });
		const app = view.container.querySelector("#dv-app") as HTMLElement;
		const toggle = byRole("button", /^Can't run on devices · \d events?$/, app);
		expect(toggle.getAttribute("aria-expanded")).toBe("false");
		const before = app.querySelectorAll("tr[data-event]").length;
		await click(toggle);
		expect(toggle.getAttribute("aria-expanded")).toBe("true");
		expect(app.querySelectorAll("tr[data-event]").length).toBeGreaterThan(
			before,
		);
	});

	test("an app that doesn't run here: the block stands alone and offers to deploy", async () => {
		const view = await open(IDS.studio, { app: APPS.invoiceAi });
		const app = view.container.querySelector("#dv-app") as HTMLElement;
		expect(text(app)).toContain("0 services");
		expect(text(app)).toContain(
			"Invoice AI is an online app. Deployed here, studio-mac-mini runs it online at the version you deploy from the hub.",
		);
		expect(view.container.querySelector("#services-block")).toBeNull();
		expect(
			app.querySelectorAll("[data-matrix-cell=not_served]").length,
		).toBeGreaterThan(0);
	});

	test("a locked device: the lock instead of a matrix, and the how-to waits for the unlock", async () => {
		const view = await open(IDS.lab, { app: APPS.invoiceAi });
		const app = view.container.querySelector("#dv-app") as HTMLElement;
		expect(app.querySelector("[data-gate=locked]")).not.toBeNull();
		expect(text(app)).toContain(
			"Unlock lab-gpu-02 to see which Invoice AI services run there.",
		);
		expect(text(app)).toContain(
			"After you unlock lab-gpu-02, Deploy Invoice AI here…",
		);
		expect(app.querySelector("[data-matrix-cell]")).toBeNull();
		expect(view.container.querySelector("#services-block")).toBeNull();
	});
});

describe("older hub and older agent", () => {
	test("older agent: rows without event lists or restart counts, and no newer command", async () => {
		const view = await open(IDS.edge, { agentFeatures: {} });
		expect(view.container.querySelectorAll("tr[data-service]").length).toBe(3);
		expect(text(row(view, "support-bot"))).toContain(
			"Events unknown: the status snapshot has no event list",
		);
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
		const sent = commandTypes(view);
		for (const newer of [
			"host_operation",
			"rollout_history",
			"operations",
			"metrics_history",
		])
			expect(sent).not.toContain(newer);
		expect(text(view.container)).not.toMatch(MACHINE);
	});

	test("older hub: the table renders, cloud badges are left out, and new routes are asked once", async () => {
		const seed = sampleFleet();
		const view = await open(IDS.edge, { seed, hubVersion: "old" });
		expect(view.container.querySelectorAll("tr[data-service]").length).toBe(3);
		expect(text(row(view, "invoice-extractor"))).not.toContain(
			"Cloud access ·",
		);
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
		expect(view.container.querySelector("[data-kind=error]")).toBeNull();
		expect(
			view.fake.api.sent("GET", "devices/resource-summary").length,
		).toBeLessThanOrEqual(1);
		expect(
			view.fake.api.sent("GET", /device-placements$/).length,
		).toBeLessThanOrEqual(3);
	});
});
