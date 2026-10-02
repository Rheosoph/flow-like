import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	advance,
	allByRole,
	byRole,
	click,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { DeviceView } from "./device-test-kit";

const dom = installDom();
const kit = await import("./device-test-kit");
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

const block = (view: DeviceView, id: string) => {
	const found = view.container.querySelector<HTMLElement>(`#device-${id}`);
	if (!found) throw new Error(`No Overview block "${id}"`);
	return found;
};
const hasBlock = (view: DeviceView, id: string) =>
	view.container.querySelector(`#device-${id}`) !== null;

const GIB = 1024 ** 3;

function metricsSample(at: number, cpu: number) {
	return {
		sequence: at,
		timestamp: at,
		data: {
			cpu_percent: cpu,
			logical_cpus: 8,
			memory_used_bytes: 5.7 * GIB,
			memory_total_bytes: 16 * GIB,
			storage_volume: { available_bytes: 281 * GIB, total_bytes: 477 * GIB },
			network: {
				received_bytes: 1_258_291,
				transmitted_bytes: 841_728,
				interfaces: 2,
			},
			sample_seconds: 5,
		},
	};
}

/** The golden fleet with an encrypted metrics snapshot for edge-berlin-01, `ageS` seconds old. */
function seedWithMetrics(ageS: number) {
	const seed = sampleFleet();
	const at = seed.now - ageS;
	seed.fleet[IDS.edge].metrics = [
		{
			scope: { kind: "device" },
			observedAt: at,
			sample: {
				records: [
					metricsSample(at - 60, 21.1),
					metricsSample(at - 30, 22.8),
					metricsSample(at, 23.4),
				],
			},
		},
	];
	return seed;
}

describe("On this device", () => {
	test("lists what needs you, most urgent first, with the action that resolves each", async () => {
		const view = await openDevice(IDS.edge);
		const attention = block(view, "attention");
		const items = Array.from(
			attention.querySelectorAll<HTMLElement>("[data-attention]"),
		);
		expect(items.map((item) => item.getAttribute("data-sev"))).toEqual([
			"warning",
			"warning",
			"notice",
			"info",
			"info",
		]);
		expect(text(items[0])).toContain(
			"Certificate internal-mqtt on edge-berlin-01 expires in 5 days",
		);
		expect(byRole("button", "Fix renewal", attention)).toBeTruthy();
		expect(text(attention)).toMatch(/^On this device3/);
		expect(attention.querySelector("[data-stamp]")).not.toBeNull();
		expect(text(view.container)).not.toMatch(MACHINE);
	});

	test("nothing open on a locked device is an all-clear only as far as the hub knows, never empty", async () => {
		const view = await openDevice(IDS.lab);
		const attention = block(view, "attention");
		expect(text(attention)).toContain(
			"Nothing on lab-gpu-02 needs you, as far as the hub knows.",
		);
		expect(text(attention)).not.toContain("needs you right now");
		expect(
			attention.querySelector('[data-stamp][data-src="hub"]'),
		).not.toBeNull();
		expect(attention.querySelector("[data-kind=empty]")).toBeNull();
	});

	test("nothing open on a device that is read here is a plain all-clear", async () => {
		const seed = sampleFleet();
		const view = await openDevice(IDS.edge, { seed, app: APPS.supportPortal });
		expect(text(block(view, "attention"))).toContain(
			"Nothing about Support Portal on edge-berlin-01 needs you right now.",
		);
	});

	test("a person is named, or neutrally while the directory has no name: never by account id", async () => {
		const view = await openDevice(IDS.edge);
		const unnamed = text(block(view, "attention"));
		expect(unnamed).toContain("One person's access to edge-berlin-01 ends");
		expect(unnamed).not.toMatch(/\busr_\w+/);
		await kit.resetDevices();

		const fake = await createFakeWorkspace();
		const person = async (id: string) => ({ id, name: "Mira Novak" });
		const named = await openDevice(IDS.edge, {
			fake,
			backend: {
				userState: {
					getProfile: async () => fake.profile,
					getInfo: async () => ({ id: fake.hub.me, dev_mode: false }),
					updateUser: async () => undefined,
					lookupUser: person,
					lookupUsers: async (ids: string[]) => Promise.all(ids.map(person)),
				} as never,
			},
		});
		for (
			let round = 0;
			round < 40 &&
			!text(block(named, "attention")).includes("Mira Novak's access");
			round++
		)
			await advance(25);
		expect(text(block(named, "attention"))).toContain(
			"Mira Novak's access to edge-berlin-01 ends",
		);
	});

	test("in an app: only that app's items, and a way to the whole device", async () => {
		const view = await openDevice(IDS.edge, { app: APPS.invoiceAi });
		const attention = block(view, "attention");
		expect(text(attention)).toContain("Invoice AI on this device");
		expect(text(attention)).not.toContain("internal-mqtt");
		expect(text(attention)).toMatch(
			/\d items? about the rest of edge-berlin-01 (isn't|aren't) shown\./,
		);
		await click(byRole("button", "Show whole device", attention));
		expect(lastNavigation(view)?.[1]).toContain("/settings/devices?");
		expect(lastNavigation(view)?.[1]).toContain(`device=${IDS.edge}`);
	});
});

describe("Services summary", () => {
	test("each service with requested → actual and ready instances; the foot opens the Services tab", async () => {
		const view = await openDevice(IDS.edge);
		const services = block(view, "services-summary");
		const names = allByRole("link", undefined, services)
			.filter((link) => link.getAttribute("href")?.includes("service="))
			.map((link) => text(link));
		expect(names).toEqual(["invoice-extractor", "nightly-sync", "support-bot"]);
		expect(text(services)).toContain("2 of 2 ready");
		expect(text(services)).toContain("0 of 1 ready");
		expect(
			services.querySelector('[data-dvo][data-conv="update_in_progress"]'),
		).not.toBeNull();
		expect(
			byRole("link", "Invoice AI", services).getAttribute("href"),
		).toContain(`id=${APPS.invoiceAi}`);
		await click(byRole("button", "All 3 services", services));
		await view.settle();
		expect(lastNavigation(view)).toEqual([
			"replace",
			expect.stringContaining("tab=services"),
		]);
	});

	test("locked without earlier data: the lock and its reason, not an empty list", async () => {
		const view = await openDevice(IDS.edge, { unlock: "none" });
		const services = block(view, "services-summary");
		expect(services.querySelector("[data-gate=locked]")).not.toBeNull();
		expect(text(services)).toContain(
			"Unlock edge-berlin-01 to see its services.",
		);
		expect(services.querySelector("[data-kind=empty]")).toBeNull();
	});

	test("in an app that doesn't run here: says so and counts the other apps' services", async () => {
		const view = await openDevice(IDS.studio, { app: APPS.invoiceAi });
		const services = block(view, "services-summary");
		expect(text(services)).toContain("Invoice AI services");
		expect(text(services)).toContain(
			"Invoice AI doesn't run on studio-mac-mini",
		);
		expect(text(services)).toContain("1 service of other apps runs here.");
	});
});

describe("Resources", () => {
	test("without samples: not loaded, with the reason", async () => {
		const view = await openDevice(IDS.edge);
		const resources = block(view, "resources");
		expect(resources.querySelector("[data-kind=notloaded]")).not.toBeNull();
		expect(text(resources)).toContain(
			"This device hasn't sent resource samples yet.",
		);
	});

	test("a current snapshot shows CPU, memory, disk and network with its trend", async () => {
		const view = await openDevice(IDS.edge, { seed: seedWithMetrics(20) });
		const resources = block(view, "resources");
		const metrics = Array.from(
			resources.querySelectorAll<HTMLElement>("[data-metric]"),
		).map((metric) => text(metric));
		expect(metrics).toHaveLength(4);
		expect(metrics[0]).toContain(
			"CPU23.4%8 logical CPUs · 100 % = all of them",
		);
		expect(metrics[1]).toContain("Memory5.7of 16.0 GiB36 % used");
		expect(metrics[2]).toContain("Agent's data disk281GiB free of 477");
		expect(metrics[3]).toContain("Network1.2MiB received · 822.0 KiB sent");
		expect(metrics[3]).toContain("not a billing figure");
		expect(resources.querySelectorAll("[data-sparkline]").length).toBe(4);
		expect(
			resources
				.querySelector('[data-stamp][data-src="snap"]')
				?.getAttribute("data-age"),
		).toBe("current");
	});

	test("a snapshot older than 75 s keeps its data and is marked last known", async () => {
		const view = await openDevice(IDS.edge, { seed: seedWithMetrics(120) });
		const resources = block(view, "resources");
		expect(resources.querySelectorAll("[data-metric]").length).toBe(4);
		expect(text(resources)).toContain("23.4");
		expect(
			resources
				.querySelector('[data-stamp][data-src="snap"]')
				?.getAttribute("data-age"),
		).toBe("lastknown");
		expect(resources.querySelector("[data-kind=error]")).toBeNull();
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
	});
});

describe("Diagnose on the device", () => {
	test("appears for an offline device with the commands only the device can answer", async () => {
		const view = await openDevice(IDS.warehouse);
		const diagnose = block(view, "diagnose");
		const commands = Array.from(
			diagnose.querySelectorAll("[data-command] code"),
			(code) => code.textContent,
		);
		expect(commands).toEqual([
			"flow-like-standalone status",
			"flow-like-standalone service-status",
			"flow-like-standalone recover-enrollment",
			"./flow-like-standalone install-service",
		]);
		expect(
			diagnose.querySelector('[data-stamp][data-src="device"]'),
		).not.toBeNull();
		await click(byRole("button", "Open full diagnosis", diagnose));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "diagnose",
			deviceId: IDS.warehouse,
		});
	});

	test("is absent while the device checks in and the hub refused nothing", async () => {
		const view = await openDevice(IDS.edge);
		expect(hasBlock(view, "diagnose")).toBe(false);
	});

	test("names the refusal the hub recorded: a clock that is ahead", async () => {
		const seed = sampleFleet();
		const row = seed.devices.find(
			(device) => device.device_id === IDS.warehouse,
		);
		if (!row) throw new Error("the sample has no warehouse device");
		row.auth_rejection = {
			code: "clock_skew",
			skew_seconds: 540,
			count: 6,
			first_at: seed.now - 3_000,
			last_at: seed.now - 60,
		};
		const view = await openDevice(IDS.warehouse, { seed });
		expect(text(block(view, "diagnose"))).toMatch(
			/The hub refused warehouse-pi's last 6 check-ins since .+ because its clock is about 9 min ahead\. Set the clock on the device\./,
		);
	});

	test("older hub: the commands alone, without a refusal sentence or an error", async () => {
		const view = await openDevice(IDS.warehouse, { hubVersion: "old" });
		const diagnose = block(view, "diagnose");
		expect(text(diagnose)).not.toContain("The hub refused");
		expect(diagnose.querySelectorAll("[data-command]").length).toBe(4);
		expect(view.container.querySelector("[data-kind=error]")).toBeNull();
	});
});

describe("Trust, registration, certificates and access", () => {
	test("the compact trust chain states each link and leads to the Keys tab", async () => {
		const view = await openDevice(IDS.edge);
		const trust = block(view, "trust");
		const states = Array.from(trust.querySelectorAll("[data-state]"), (link) =>
			link.getAttribute("data-state"),
		);
		expect(states).toEqual(["good", "good", "good", "warning"]);
		expect(text(trust)).toContain(
			"Owner keys · unlocked · backed up to your account (v3)",
		);
		expect(text(trust)).toContain("Matches the hub ·");
		expect(text(trust)).toContain("Active on device · 2 people · expire");
		expect(text(trust)).toContain("2 · the next one expires in 5 days");
		expect(trust.querySelector("[data-join]")).toBeNull();
		await click(byRole("button", "Keys and trust details", trust));
		expect(lastNavigation(view)?.[1]).toContain("tab=keys");
	});

	test("a changed identity: the chain says the keys stay closed for it, not that the browser is at fault", async () => {
		const fake = await createFakeWorkspace(undefined, { unlock: "none" });
		const row = fake.hub.rows.get(IDS.edge);
		if (!row) throw new Error("the sample has no edge device");
		row.identity = fakeKeys.identity("someone-else");
		const view = await openDevice(IDS.edge, { fake });
		await view.settle();
		const trust = text(block(view, "trust"));
		expect(trust).toContain(
			"Owner keys · closed until the identity is confirmed",
		);
		expect(trust).toContain(
			"Doesn't match the keys you trusted. Management is blocked.",
		);
		expect(text(view.container).toLowerCase()).not.toContain(
			"browser can't protect keys",
		);
	});

	test("registration facts: when, last check-in with what it proves, owner, ID and the advanced details", async () => {
		const view = await openDevice(IDS.edge);
		const registration = block(view, "registration");
		const page = text(registration);
		expect(page).toContain("Registered");
		expect(page).toContain("Last check-in");
		expect(page).toContain("It doesn't prove a live connection works.");
		expect(page).toContain("OwnerYOYou");
		expect(page).toContain("Key generation1");
		expect(page).toContain("increases only when a device is revoked");
		expect(
			registration.querySelector('[data-stamp][data-src="hub"]'),
		).not.toBeNull();
		await click(byRole("button", "Copy device ID", registration));
		expect(dom.clipboard).toContain(IDS.edge);
	});

	test("certificates from the hub's public inventory, named after a live read", async () => {
		const view = await openDevice(IDS.edge);
		const certificates = block(view, "certificates");
		expect(text(certificates)).toContain("edge-api");
		expect(text(certificates)).toContain("internal-mqtt");
		expect(text(certificates)).toContain("device confirmed");
		await click(byRole("button", "Open certificates", certificates));
		expect(lastNavigation(view)?.[1]).toContain("tab=certificates");
	});

	test("locked: certificates show by ID and say the name needs a live read", async () => {
		const view = await openDevice(IDS.edge, { unlock: "none" });
		const certificates = block(view, "certificates");
		expect(text(certificates)).not.toContain("internal-mqtt");
		expect(text(certificates)).toContain("Name shows after a live read.");
		expect(certificates.querySelectorAll("li").length).toBe(2);
	});

	test("access: how many people, saved against applied rules, and when the rules expire", async () => {
		const view = await openDevice(IDS.edge);
		const access = block(view, "access");
		expect(text(access)).toContain("Shared with 2 people");
		expect(
			access.querySelector('[data-dvo="versions"]')?.getAttribute("aria-label"),
		).toBe("Access rules: saved v5, device has v5");
		expect(text(access)).toContain("rules expire");
		await click(byRole("button", "Open access", access));
		expect(lastNavigation(view)?.[1]).toContain("tab=access");
	});

	test("a recipient reads their own access in the summary", async () => {
		const view = await openDevice(IDS.lab);
		expect(text(block(view, "access"))).toMatch(
			/^Access.*Your access: Custom · 6 permissions · App Invoice AI · ends in/,
		);
	});
});

describe("Recent activity", () => {
	test("what the device recorded, newest first, over the live connection", async () => {
		const fake = await createFakeWorkspace(undefined, { unlock: "none" });
		const agent = fake.agent(IDS.edge);
		agent.record(
			"messages:device",
			{
				kind: "operation",
				state: "completed",
				placement_id: "support-bot",
				source_id: "a98f44f5-f313-4376-a86c-9b8c14a8161c",
			},
			"message",
		);
		agent.record(
			"messages:device",
			{
				kind: "replica",
				state: "starting",
				placement_id: "invoice-extractor",
				replica_slot: 0,
				config_revision: 12,
			},
			"message",
		);
		await fake.unlock(IDS.edge, { connectLive: true });
		const view = await openDevice(IDS.edge, { fake });
		const recent = block(view, "recent");
		for (
			let round = 0;
			round < 40 && !text(recent).includes("Instance #0");
			round++
		)
			await advance(25);
		const page = text(recent);
		// The same sentences as the Activity tab's timeline, with the service as a link.
		expect(page).toContain(
			"Instance #0 of invoice-extractor is starting with settings v12.",
		);
		expect(page).toContain("A command for support-bot finished.");
		expect(page.indexOf("Instance #0")).toBeLessThan(
			page.indexOf("A command for support-bot"),
		);
		expect(
			byRole("link", "invoice-extractor", recent).getAttribute("href"),
		).toContain("service=invoice-extractor");
		const entries = recent.querySelectorAll("[data-timeline] li[data-kind]");
		expect(entries.length).toBeGreaterThanOrEqual(2);
		expect(entries.length).toBeLessThanOrEqual(5);
		expect(page).not.toMatch(MACHINE);
		await click(byRole("button", "All activity", recent));
		expect(lastNavigation(view)?.[1]).toContain("tab=activity");
	});

	test("locked: says so with the way to unlock, and reads nothing", async () => {
		const view = await openDevice(IDS.edge, { unlock: "none" });
		const recent = block(view, "recent");
		expect(recent.querySelector("[data-kind=locked]")).not.toBeNull();
		expect(text(recent)).toContain(
			"Activity is cleared when you lock. Unlock to read it again.",
		);
		expect(recent.querySelector("[data-kind=empty]")).toBeNull();
		expect(kit.commandTypes(view)).toEqual([]);
	});
});

describe("who sees which blocks", () => {
	test("a revoked device: no resources and no activity, certificates aren't read", async () => {
		const view = await openDevice(IDS.oldKiosk);
		expect(hasBlock(view, "resources")).toBe(false);
		expect(hasBlock(view, "recent")).toBe(false);
		expect(text(block(view, "certificates"))).toContain(
			"Revoked devices aren't read.",
		);
		expect(text(block(view, "services-summary"))).toContain(
			"Revoked devices aren't read.",
		);
		expect(view.container.querySelector("[data-kind=empty]")).toBeNull();
	});

	test("cloud approvals only: what needs you, registration and access", async () => {
		const view = await openDevice(IDS.partner);
		expect(hasBlock(view, "attention")).toBe(true);
		expect(hasBlock(view, "registration")).toBe(true);
		expect(hasBlock(view, "access")).toBe(true);
		for (const hidden of [
			"services-summary",
			"resources",
			"trust",
			"certificates",
			"recent",
		])
			expect(hasBlock(view, hidden)).toBe(false);
		expect(primaries()).toBe(0);
	});
});
