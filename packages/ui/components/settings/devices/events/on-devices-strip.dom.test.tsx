import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { sampleDevices } from "../../../../lib/device-management/model/__fixtures__/apps";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";
import type { EventsDevicesBlock, EventsDevicesValue } from "./events-devices";
import type { SampleApp, SampleValueOptions } from "./events-test-kit";

const dom = installDom();
const { EVENTS_BLOCK_ID, EventsDevicesBanner, OnDevicesStrip } = await import(
	"./on-devices-strip"
);
const { SampleEventsDevices, sampleValue } = await import("./events-test-kit");
const { useOverlayStore } = await import("../workspace/overlay-store");

afterEach(async () => {
	useOverlayStore.getState().close();
	await dom.cleanup();
});
afterAll(dom.restore);

const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|\bG\d{1,2}\b/;

async function renderStrip(
	appId: SampleApp,
	options: SampleValueOptions = {},
	edit?: (value: EventsDevicesValue) => void,
) {
	const value = sampleValue(appId, options);
	edit?.(value);
	const view = await dom.render(
		<SampleEventsDevices value={value}>
			<EventsDevicesBanner />
			<OnDevicesStrip />
		</SampleEventsDevices>,
	);
	const strip = view.container.querySelector<HTMLElement>("[data-on-devices]");
	return { ...view, value, strip, text: strip?.textContent ?? "" };
}

/** Moves every event that can run into the can't-run group. */
function noneCanRun(value: EventsDevicesValue) {
	if (!value.live) return;
	const { events } = value.live.view;
	value.live.view.events = {
		...events,
		rows: [],
		ineligible: [
			...events.ineligible,
			...events.rows.map(({ cells: _cells, ...row }) => row),
		],
	};
	value.live.rows = new Map();
}

describe("On devices strip (APP §4.2)", () => {
	test("online: how the app runs, how many events can run and where, and the coverage sentence", async () => {
		const { strip, text } = await renderStrip("app_invoice_ai");
		expect(strip?.getAttribute("aria-label")).toBe("Invoice AI on devices");
		expect(strip?.querySelector("[data-mode='online']")?.textContent).toBe(
			"Runs online",
		);
		expect(text).toContain(
			"Devices run the version you deploy from the hub. Data stays in the cloud, so they need internet.",
		);
		expect(text).toContain(
			"3 of 6 events can run on a device. 1 of them runs on 1 device you can see.",
		);
		expect(text).toContain(
			"Status from 3 of 5 devices you can see; 1 is unknown, 1 hasn't checked in yet.",
		);
		expect(text).not.toContain("when the copy arrives");
		expect(strip?.querySelector("[data-stamp][data-src='mixed']")).toBeTruthy();
		expect(text).not.toMatch(MACHINE_WORDS);
	});

	test("local-only: offline copy, and each device checks the events again", async () => {
		const { strip, text } = await renderStrip("app_support_portal");
		expect(strip?.querySelector("[data-mode='offline']")?.textContent).toBe(
			"Offline copy",
		);
		expect(text).toContain(
			"2 of 5 events can run on a device. 2 of them run on 1 device you can see. Each device checks them again when the copy arrives.",
		);
	});

	test("never deployed: “on a device you can see” only while a device is unknown", async () => {
		const locked = await renderStrip("app_visitor_checkin");
		expect(locked.text).toContain(
			"None of them is on a device you can see yet.",
		);
		await locked.unmount();
		const open = await renderStrip("app_visitor_checkin", {
			labUnlocked: true,
		});
		expect(open.text).toContain("None of them is on a device yet.");
		expect(open.text).toContain(
			"Status from 4 of 5 devices you can see; 1 hasn't checked in yet.",
		);
	});

	test("one event: the count sentence is singular", async () => {
		const { text } = await renderStrip("app_invoice_ai", {}, (value) => {
			if (!value.live) return;
			const { events } = value.live.view;
			value.live.view.events = {
				...events,
				rows: events.rows.slice(0, 1),
				ineligible: [],
			};
		});
		expect(text).toContain("1 of 1 event can run on a device.");
	});

	test("Unlock names the one locked device and asks the overlay host", async () => {
		const { strip } = await renderStrip("app_invoice_ai");
		await click(byRole("button", "Unlock lab-gpu-02…", strip ?? undefined));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: "lab-gpu-02",
		});
	});

	test("several locked devices: Unlock N… opens Unlock several; never-checked-in ones don't count", async () => {
		const devices = sampleDevices().map((device) =>
			device.id === "warehouse-pi"
				? {
						...device,
						keyState: "locked" as const,
						services: { state: "locked" as const },
					}
				: device,
		);
		const { strip } = await renderStrip("app_invoice_ai", { devices });
		await click(byRole("button", "Unlock 2…", strip ?? undefined));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock_several",
		});
	});

	test("nothing locked: no Unlock button", async () => {
		const { strip } = await renderStrip("app_invoice_ai", {
			labUnlocked: true,
		});
		expect(queryByRole("button", /^Unlock/, strip ?? undefined)).toBeNull();
	});

	test("Deploy to devices… and Open in Devices lead into the app's device pages", async () => {
		const { strip } = await renderStrip("app_invoice_ai");
		expect(
			byRole("link", "Deploy to devices…", strip ?? undefined).getAttribute(
				"href",
			),
		).toBe("/library/config/devices?id=app_invoice_ai&flow=deploy&mode=new");
		expect(
			byRole("link", "Open in Devices", strip ?? undefined).getAttribute(
				"href",
			),
		).toBe("/library/config/devices?id=app_invoice_ai");
	});

	test("Deploy to devices… is off with the block cause, described by the banner (R7)", async () => {
		const opened: string[] = [];
		const { strip } = await renderStrip("app_invoice_ai", {
			canDeploy: false,
			opened,
		});
		const deploy = byRole("button", "Deploy to devices…", strip ?? undefined);
		expect(deploy.getAttribute("aria-disabled")).toBe("true");
		expect(deploy.getAttribute("aria-describedby")).toBe(EVENTS_BLOCK_ID);
		expect(document.getElementById(EVENTS_BLOCK_ID)?.textContent).toContain(
			"You have no device that can take a deploy right now",
		);
		await click(deploy);
		expect(opened).toEqual([]);
	});

	test("Deploy to devices… is off when no event can run, described by the count line", async () => {
		const { strip, text } = await renderStrip("app_invoice_ai", {}, noneCanRun);
		expect(text).toContain("0 of 6 events can run on a device.");
		const deploy = byRole("button", "Deploy to devices…", strip ?? undefined);
		expect(deploy.getAttribute("aria-disabled")).toBe("true");
		expect(deploy.getAttribute("title")).toBe(
			"None of Invoice AI's events can run on a device",
		);
		const describedBy = deploy.getAttribute("aria-describedby") ?? "";
		expect(document.getElementById(describedBy)?.textContent).toContain(
			"0 of 6 events can run on a device.",
		);
	});

	test("Online or offline? opens the explainer", async () => {
		let asked = 0;
		const { strip } = await renderStrip("app_invoice_ai", {
			value: { explainMode: () => asked++ },
		});
		await click(byRole("button", "Online or offline?", strip ?? undefined));
		expect(asked).toBe(1);
	});

	test("a failing device list keeps the data and says so in the stamp (R5)", async () => {
		const { strip, text } = await renderStrip("app_invoice_ai", {}, (value) => {
			if (!value.live) return;
			value.live.hub = {
				src: "hub",
				age: "error",
				at: 1_790_769_587,
				dataFrom: 1_790_769_587,
				error: { code: "network" },
			};
		});
		expect(strip?.querySelector("[data-stamp][data-age='error']")).toBeTruthy();
		expect(text).toContain("3 of 6 events can run on a device.");
	});

	test("while devices load the strip says so; without device data it is not rendered", async () => {
		const loading = await renderStrip("app_invoice_ai", { status: "loading" });
		expect(loading.text).toBe("On devicesChecking devices…");
		expect(loading.strip?.getAttribute("aria-busy")).toBe("true");
		await loading.unmount();
		for (const status of ["signed_out", "hub_off", "blind", "error"] as const) {
			const view = await renderStrip("app_invoice_ai", { status });
			expect(view.strip).toBeNull();
			await view.unmount();
		}
	});

	test("no primary button (R2)", async () => {
		await renderStrip("app_invoice_ai");
		expect(document.querySelectorAll("[data-dv-primary]")).toHaveLength(0);
	});
});

describe("the one banner above the sections (APP §4.5)", () => {
	const banner = () => document.getElementById(EVENTS_BLOCK_ID);

	async function renderBlock(
		block: EventsDevicesBlock,
		value: Partial<EventsDevicesValue> = {},
	) {
		const status =
			block === "no_target" ? "ready" : (block as SampleValueOptions["status"]);
		return renderStrip("app_invoice_ai", {
			status,
			canDeploy: block !== "no_target",
			value,
		});
	}

	test("signed out: asks to sign in and starts the host's sign-in", async () => {
		let signIns = 0;
		await renderBlock("signed_out", { signIn: () => signIns++ });
		expect(banner()?.textContent).toContain("Run on a device… isn't available");
		expect(banner()?.textContent).toContain(
			"Devices belong to your Flow-Like account. Sign in to see where these events run and to deploy them.",
		);
		await click(byRole("button", "Sign in"));
		expect(signIns).toBe(1);
	});

	test("a token that can't manage devices says so", async () => {
		await renderBlock("token");
		expect(banner()?.textContent).toContain(
			"Your access token can't list or manage devices.",
		);
		expect(queryByRole("button", "Sign in")).toBeNull();
	});

	test("device support off: names it and links to the hub's status", async () => {
		await renderBlock("hub_off");
		expect(banner()?.textContent).toContain(
			"Device support is off on this hub",
		);
		expect(banner()?.textContent).toContain(
			"Events keep working in Flow-Like as before.",
		);
		expect(byRole("link", "Open hub status").getAttribute("href")).toBe(
			"/settings/devices?view=hub",
		);
	});

	test("devices couldn't be read: the cause and Retry", async () => {
		let retries = 0;
		await renderBlock("error", {
			problem: { code: "network", retry: () => retries++ },
		});
		expect(banner()?.textContent).toContain("Devices couldn't be checked");
		expect(banner()?.textContent).toContain("The hub can't be reached.");
		await click(byRole("button", "Retry"));
		expect(retries).toBe(1);
	});

	test("no device can take a deploy: says why and opens the fleet", async () => {
		await renderBlock("no_target");
		expect(banner()?.textContent).toContain(
			"You have no device that can take a deploy right now: each one is offline, revoked, locked to another app or has no keys on this computer.",
		);
		expect(byRole("link", "Open devices").getAttribute("href")).toBe(
			"/settings/devices",
		);
	});

	test("no device at all: says to set one up", async () => {
		await renderStrip("app_invoice_ai", { devices: [], canDeploy: false });
		expect(banner()?.textContent).toContain(
			"You have no device yet. Set one up in Devices, then run these events on it.",
		);
	});

	test("a role without flows is told by the page's own notice: no banner here", async () => {
		await renderBlock("blind");
		expect(banner()).toBeNull();
	});

	test("nothing blocks: no banner", async () => {
		await renderStrip("app_invoice_ai");
		expect(banner()).toBeNull();
		expect(document.querySelectorAll("[data-events-block]")).toHaveLength(0);
	});

	test("no machine vocabulary in any banner (R3)", async () => {
		for (const block of [
			"signed_out",
			"token",
			"hub_off",
			"error",
			"no_target",
		] as const) {
			const view = await renderBlock(block, {
				problem: { code: "server_error" },
			});
			expect(banner()?.textContent).not.toMatch(MACHINE_WORDS);
			await view.unmount();
		}
	});
});
