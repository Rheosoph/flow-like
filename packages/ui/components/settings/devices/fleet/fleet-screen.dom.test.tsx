import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import * as sample from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { generateFleet } from "../../../../lib/device-management/model/__fixtures__/sample-fleet-200";
import {
	advance,
	allByRole,
	byRole,
	byText,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { DeviceSeed } from "../testing/fake-device-api";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { act } = await import("react");
const { FleetScreen } = await import("./fleet-screen");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useActivityTray } = await import("../shell/activity-tray");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { fakeDeviceApi } = await import("../testing/fake-device-api");

const { SAMPLE_IDS, SAMPLE_ME, SAMPLE_NOW } = sample;

/** Command variants this work added to the agent (plan §3.4.3): an older agent must never be asked. */
const NEWER_COMMANDS =
	"host_operation rollout_history operations metrics_history offline_queue_operations offline_queue_lookup".split(
		" ",
	);

async function resetAfterTest() {
	await cleanupDevices();
	await dom.cleanup();
	useActivityTray.getState().setOpen(false);
	useOverlayStore.getState().close();
}
afterEach(resetAfterTest);
afterAll(dom.restore);

/** A 200-device fleet takes several seconds to seed and mount on a busy machine. */
const SCALE_TIMEOUT_MS = 30_000;

const FACTORY_SETUP = "c382dd52-1611-4477-b983-4e1e1417b669";
const QUIET = "11111111-2222-4333-8444-555555555555";

const CLOCK = "\\d\\d:\\d\\d:\\d\\d";
const FAILED_REFRESH = new RegExp(
	`Couldn't refresh at ${CLOCK}: The hub is limiting requests\\. It retries by itself\\. Showing data from ${CLOCK}\\.`,
);

/** No snake_case wire value, gate code or pre-flight code may reach the screen (R3). */
const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|\bG\d{1,2}\b|\bD\d\b/;

/** The screen under the area's own route, as the screen switch mounts it. */
function Routed() {
	const { route, scope } = useDevicesRoute();
	return <FleetScreen route={route} scope={scope} />;
}

async function mountFleet(options: MountDevicesOptions = {}) {
	const mounted = await mountDevices(<Routed />, options);
	// Owner names are looked up in one batch 20 ms after the rows render.
	await advance(40);
	await mounted.settle();
	return mounted;
}

const primaries = (root: ParentNode) =>
	root.querySelectorAll("[data-dv-primary]").length;

const hrefOf = (name: string | RegExp) =>
	byRole("link", name).getAttribute("href") ?? "";

/** One of the six annunciator windows (critical, attention, unknown, healthy, offline, revoked). */
function healthWindow(name: string) {
	const group = byRole("group", /^Fleet health/);
	return group.querySelector(`[data-window="${name}"]`) as HTMLElement;
}

function windowText(name: string) {
	return healthWindow(name).textContent ?? "";
}

/** One owned device that runs what was asked and has nothing open. */
function quietFleet(): DeviceSeed {
	const seed = sample.emptyInput();
	const seenAt = SAMPLE_NOW - 30;
	const bootId = "b00t000000000001";
	const running = sample.placement({
		id: "quiet-service",
		project_id: "app_support_portal",
		desired_state: "running",
		observed_state: "running",
	});
	const row = sample.deviceRow({
		device_id: QUIET,
		owner_id: SAMPLE_ME,
		name: "quiet-box",
		status: "active",
		registered_at: SAMPLE_NOW - 30 * 86_400,
		last_seen_at: seenAt,
		relationship: "owner",
	});
	seed.hub = { state: "on", serverTime: SAMPLE_NOW };
	seed.devices.push(row);
	seed.keys.push(sample.keySession(QUIET, "unlocked"));
	seed.local.vaults.push(sample.vault(QUIET));
	seed.local.backups[QUIET] = { localRevision: 1, pending: false };
	seed.accountBackups[QUIET] = { revision: 1 };
	seed.certInventory[QUIET] = {
		revision: 1,
		updated_at: SAMPLE_NOW - 600,
		certificates: [],
	};
	const observations = [sample.observation(QUIET, [running], seenAt, bootId)];
	const status = { observations, observedAt: seenAt, bootId, sequence: 3 };
	seed.fleet[QUIET] = sample.fleetState(QUIET, { status });
	return seed;
}

describe("FleetScreen header", () => {
	test("names the hub and the account's devices and offers the page actions with one coral", async () => {
		const { container } = await mountFleet();
		expect(byRole("heading", "Fleet overview")).toBeTruthy();
		expect(container.textContent).toContain(
			"7 devices on hub.test · 1 shared with you",
		);
		expect(byRole("group", "Scope")).toBeTruthy();
		expect(byRole("button", "Refresh")).toBeTruthy();
		expect(hrefOf("Request shared access")).toBe(
			"/settings/devices?view=access&tab=shared&action=request",
		);
		const setup = byRole("link", "Set up a device");
		expect(setup.getAttribute("href")).toBe("/settings/devices?flow=setup");
		expect(setup.hasAttribute("data-dv-primary")).toBe(true);
		expect(primaries(container)).toBe(1);
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
	});

	test("Refresh reads the list again and leaves its result until dismissed", async () => {
		const { fake, container } = await mountFleet();
		const before = fake.api.sent("GET", "devices").length;
		await click(byRole("button", "Refresh"));
		expect(fake.api.sent("GET", "devices").length).toBe(before + 1);
		const result = byText(
			/^Checked at \d\d:\d\d:\d\d\. The device list is current\.$/,
		);
		expect(result).toBeTruthy();
		await click(byRole("button", "Dismiss", container));
		expect(container.textContent).not.toContain("The device list is current.");
	});

	test("a failed refresh keeps the rows and says from when they are", async () => {
		const { fake, container, settle } = await mountFleet();
		// A refusal is a verdict: the list isn't retried, so the result is immediate.
		fake.api.fail(
			{ method: "GET", path: "devices" },
			new ApiResponseError({
				status: 429,
				code: "RATE_LIMITED",
				message: "Too many requests",
			}),
		);
		await click(byRole("button", "Refresh"));
		await settle();
		expect(container.textContent).toMatch(FAILED_REFRESH);
		expect(
			container.querySelector(`tr[data-device="${SAMPLE_IDS.edge}"]`),
		).not.toBeNull();
		expect(queryByRole("alert", undefined, container)).toBeNull();
	});
});

describe("FleetScreen triage blocks", () => {
	test("the headline and the six windows sum up the fleet; a window filters the table", async () => {
		const { container, navigations } = await mountFleet();
		expect(container.querySelector("[data-headline]")?.textContent).toContain(
			"warehouse-pi needs you now.",
		);
		expect(windowText("critical")).toBe("1Criticalwarehouse-pi");
		expect(windowText("attention")).toStartWith("3Needs attention");
		expect(windowText("unknown")).toContain("lab-gpu-02 · locked");
		expect(windowText("healthy")).toContain("none right now");
		expect(windowText("offline")).toContain("warehouse-pi · also critical");
		expect(windowText("revoked")).toContain("1 still billed to you");
		expect(container.textContent).toContain(
			"Device slots 6 of 100 · 1 unused setup package · setup packages today 1 of 220.",
		);

		await click(healthWindow("critical"));
		expect(navigations.at(-1)).toEqual({
			mode: "replace",
			href: "/settings/devices?filter=critical",
		});
		expect(healthWindow("critical").getAttribute("aria-pressed")).toBe("true");
		expect(container.querySelectorAll("tr[data-device]").length).toBe(1);

		await click(healthWindow("critical"));
		expect(navigations.at(-1)).toEqual({
			mode: "replace",
			href: "/settings/devices",
		});
		expect(container.querySelectorAll("tr[data-device]").length).toBe(5);
	});

	test("Needs you shows seven items in tiers, then all fifteen", async () => {
		const { container } = await mountFleet();
		const block = container.querySelector("#devices-needs-you") as HTMLElement;
		expect(block.textContent).toContain("2 critical · 8 warnings · 5 notices");
		expect(block.querySelector('[data-tier="now"]')).not.toBeNull();
		expect(block.querySelector('[data-tier="soon"]')).not.toBeNull();
		expect(block.querySelectorAll("li[data-attention]").length).toBe(7);
		await click(byRole("button", "Show all 15", block));
		expect(block.querySelectorAll("li[data-attention][data-sev]").length).toBe(
			block.querySelectorAll("li[data-attention]").length,
		);
		expect(
			block.querySelectorAll(
				'li[data-sev="critical"],li[data-sev="warning"],li[data-sev="notice"]',
			).length,
		).toBe(15);
		expect(byRole("button", "Show fewer", block)).toBeTruthy();
	});

	test("an item that no longer needs you moves to Done in this session until dismissed", async () => {
		const { fake, container, settle } = await mountFleet();
		const block = container.querySelector("#devices-needs-you") as HTMLElement;
		expect(block.textContent).toContain("has never checked in");
		fake.hub.checkIn(SAMPLE_IDS.cold);
		await act(async () => {
			await fake.queryClient.refetchQueries();
		});
		await settle();
		const done = block.querySelector('[data-tier="done"]') as HTMLElement;
		expect(done.textContent).toContain("Resolved.");
		expect(done.textContent).toContain("has never checked in");
		expect(done.querySelectorAll("li[data-done]").length).toBe(1);
		await click(byRole("button", "Dismiss", done));
		expect(block.querySelector('[data-tier="done"]')).toBeNull();
	});

	test("locking the devices doesn't call the items they showed resolved", async () => {
		const { fake, container, settle } = await mountFleet();
		const block = container.querySelector("#devices-needs-you") as HTMLElement;
		expect(block.textContent).toContain("kept crashing");
		const encrypted = byText("Encrypted status").closest(
			"section",
		) as HTMLElement;
		await click(byRole("button", "Lock all", encrypted));
		await settle();
		expect(block.textContent).not.toContain("kept crashing");
		expect(block.textContent).not.toContain("Resolved.");
		expect(block.querySelector('[data-tier="done"]')).toBeNull();

		// A fact the hub states without keys still counts when it clears.
		fake.hub.checkIn(SAMPLE_IDS.cold);
		await act(async () => {
			await fake.queryClient.refetchQueries();
		});
		await settle();
		const done = block.querySelector('[data-tier="done"]') as HTMLElement;
		expect(done.querySelectorAll("li[data-done]").length).toBe(1);
		expect(done.textContent).toContain("has never checked in");
	});

	test("In progress previews three tracked operations and opens the tray", async () => {
		const { container } = await mountFleet();
		const block = byText("In progress").closest("section") as HTMLElement;
		expect(block.querySelectorAll("[data-op]").length).toBe(3);
		expect(block.textContent).toContain("1 more in Activity");
		expect(block.textContent).toContain("tracked on this computer");
		await click(byRole("button", "Open activity", block));
		expect(useActivityTray.getState().open).toBe(true);
		expect(primaries(container)).toBeLessThanOrEqual(1);
	});

	test("Encrypted status counts what is readable and lists only the devices that aren't", async () => {
		const { fake, container } = await mountFleet();
		const block = byText("Encrypted status").closest("section") as HTMLElement;
		expect(block.querySelector("[data-encrypted-summary]")?.textContent).toBe(
			"3 of 5 active devices readable · 2 live · 1 snapshot · 2 locked",
		);
		expect(
			block.querySelector("[data-meter]")?.getAttribute("aria-label"),
		).toBe("2 live, 1 from snapshots, 2 locked");
		const rows = [...block.querySelectorAll<HTMLElement>("[data-exception]")];
		expect(rows.map((row) => row.dataset.exception)).toEqual([
			SAMPLE_IDS.cold,
			SAMPLE_IDS.lab,
		]);
		expect(rows[0].textContent).toContain("no status sent yet");
		expect(rows[1].textContent).toContain("Shared-access keys");

		const writes = fake.api.writes().length;
		await click(byRole("button", "Unlock…", rows[1]));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: SAMPLE_IDS.lab,
		});
		await click(byRole("button", "Unlock 2 devices…", block));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock_several",
		});
		await click(byRole("button", "See all 5", block));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "plane",
			plane: "status",
		});
		await click(byRole("button", "Lock all", block));
		expect(
			fake.workspace.keys.list().filter((row) => row.state === "unlocked"),
		).toEqual([]);
		expect(fake.api.writes().length).toBe(writes);
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
	});
});

describe("FleetScreen pending setups", () => {
	test("lists waiting and lapsed setups with what each still allows", async () => {
		const { container } = await mountFleet();
		const block = container.querySelector(
			"#devices-pending-setups",
		) as HTMLElement;
		const rows = [...block.querySelectorAll<HTMLElement>("tr[data-setup]")];
		expect(rows.length).toBe(2);
		expect(rows[0].textContent).toContain("factory-line-3");
		expect(rows[0].textContent).toContain("Waiting for the device");
		expect(rows[0].textContent).toContain("Counts toward your limit");
		expect(
			byRole("link", "Start instructions", rows[0]).getAttribute("href"),
		).toBe(`/settings/devices?flow=setup&enrollment=${FACTORY_SETUP}&step=6`);
		expect(rows[1].textContent).toContain("test-vm");
		expect(rows[1].textContent).toContain("Expired");
		expect(rows[1].textContent).toContain("No longer works");
		expect(byRole("link", "Set up again", rows[1]).getAttribute("href")).toBe(
			"/settings/devices?flow=setup",
		);
		expect(queryByRole("button", "Cancel setup…", rows[1])).toBeNull();
	});

	test("Cancel setup… shows the consequences first and then tells the hub once", async () => {
		const { fake, container, settle } = await mountFleet();
		const block = container.querySelector(
			"#devices-pending-setups",
		) as HTMLElement;
		const before = fake.api.writes().length;
		await click(byRole("button", "Cancel setup…", block));
		const sheet = inPortal();
		expect(sheet.textContent).toContain("Cancel the setup for factory-line-3?");
		expect(sheet.textContent).toContain(
			"The setup package for factory-line-3 stops working.",
		);
		expect(sheet.textContent).toContain(
			"Frees 1 of your 10 unused-package slots.",
		);
		expect(sheet.textContent).toContain("No, this is permanent.");
		expect(fake.api.writes().length).toBe(before);

		await click(byRole("button", "Cancel setup for factory-line-3", sheet));
		await settle();
		expect(
			fake.api
				.writes()
				.slice(before)
				.map(([method, path]) => [method, path]),
		).toEqual([["DELETE", `devices/enrollments/${FACTORY_SETUP}`]]);
		expect(block.querySelector(`tr[data-setup="${FACTORY_SETUP}"]`)).toBeNull();
		expect(block.querySelector("[data-result]")).not.toBeNull();
	});
});

describe("FleetScreen states", () => {
	test("empty: what a device is, two ways to start and the hub link, with one coral", async () => {
		const { container } = await mountFleet({ seed: sample.emptyInput() });
		expect(container.textContent).toContain("No devices yet");
		expect(container.textContent).toContain(
			"A device is a computer you run Flow-Like apps on, like a Raspberry Pi, a server or a Mac.",
		);
		expect(hrefOf("Set up a device")).toBe("/settings/devices?flow=setup");
		expect(hrefOf("Request access to someone's device")).toBe(
			"/settings/devices?view=access&tab=shared&action=request",
		);
		expect(hrefOf("Check hub status")).toBe("/settings/devices?view=hub");
		expect(primaries(container)).toBe(1);
		expect(container.querySelector("table")).toBeNull();
		expect(container.querySelector('[data-kind="empty"]')).not.toBeNull();
	});

	test("all clear: the headline says so and nothing is listed as needing you", async () => {
		const { container } = await mountFleet({ seed: quietFleet() });
		expect(container.querySelector("[data-headline]")?.textContent).toContain(
			"Everything runs as you asked.",
		);
		const block = container.querySelector("#devices-needs-you") as HTMLElement;
		expect(block.textContent).toContain("Nothing needs you right now.");
		expect(
			block.querySelectorAll('li[data-attention]:not([data-sev="info"])')
				.length,
		).toBe(0);
		expect(
			container.querySelector('[data-window="healthy"]')?.textContent,
		).toContain("quiet-box");
		expect(container.textContent).toContain("All 1 active devices readable.");
		expect(primaries(container)).toBe(1);
	});

	test("keys at risk (web): the banner offers to keep the keys and to back them up", async () => {
		const { fake, container, settle } = await mountFleet({
			platform: "web",
			persistence: "denied",
		});
		const banner = byText("This browser may delete your device keys.").closest(
			"[data-tone]",
		) as HTMLElement;
		expect(banner.textContent).toContain("persistent storage");
		expect(byRole("link", "Back up keys", banner).getAttribute("href")).toBe(
			"/settings/devices?view=keys",
		);
		expect(primaries(container)).toBe(1);

		fake.browser.persistence = "persisted";
		await click(byRole("button", "Keep keys safely", banner));
		await settle();
		expect(container.textContent).toMatch(
			/The browser granted persistent storage at \d\d:\d\d:\d\d\./,
		);
		expect(container.textContent).not.toContain(
			"This browser may delete your device keys.",
		);
	});

	test("the desktop app shows no keys-at-risk banner", async () => {
		const { container } = await mountFleet({ persistence: "denied" });
		expect(container.textContent).not.toContain(
			"This browser may delete your device keys.",
		);
	});

	test("while the list is still being read there is a loading state, never an empty one", async () => {
		const api = fakeDeviceApi();
		const release = api.hold({ method: "GET", path: "devices" });
		const { container, settle } = await mountFleet({ api });
		expect(container.querySelector('[data-kind="loading"]')).not.toBeNull();
		expect(container.textContent).not.toContain("No devices yet");
		release();
		await settle();
		expect(container.querySelectorAll("tr[data-device]").length).toBe(5);
	});
});

describe("FleetScreen on an older hub and with older agents", () => {
	test("older hub: every interim renders, nothing reads as an error, and no new route is retried", async () => {
		const { fake, container } = await mountFleet({
			seed: sample.sampleFleetOlderHub(),
			hubVersion: "old",
		});
		expect(container.querySelector("[data-slots]")?.textContent).toBe(
			"This hub doesn't report device slots or how many are in use. Hub limits",
		);
		const lab = container.querySelector(
			`tr[data-device="${SAMPLE_IDS.lab}"]`,
		) as HTMLElement;
		expect(lab.textContent).toContain("Shared or cloud approvals");
		const pending = container.querySelector(
			"#devices-pending-setups",
		) as HTMLElement | null;
		if (pending)
			expect(pending.textContent).toContain("created on this computer");
		expect(queryByRole("alert", undefined, container)).toBeNull();
		expect(container.querySelector('[data-kind="error"]')).toBeNull();
		expect(container.textContent).not.toMatch(MACHINE_WORDS);

		for (const path of [
			"devices/usage",
			"devices/enrollments",
			"devices/certificate-inventory",
			"devices/resource-summary",
		])
			expect(fake.api.sent("GET", path).length).toBeLessThanOrEqual(1);
	});

	test("older hub: the Revoked group omits the dates the hub doesn't keep", async () => {
		const { container } = await mountFleet({
			seed: sample.sampleFleetOlderHub(),
			hubVersion: "old",
			search: "filter=revoked",
		});
		const kiosk = container.querySelector(
			`tr[data-device="${SAMPLE_IDS.oldKiosk}"]`,
		) as HTMLElement;
		expect(kiosk.textContent).toContain("Revoked");
		expect(kiosk.textContent).toContain("last check-in");
		expect(kiosk.textContent).not.toMatch(/Revoked\s*on /);
	});

	test("older agents: rows render from what they report and no newer command is sent", async () => {
		const { fake, container } = await mountFleet({
			seed: sample.sampleFleetOlderAgent(),
			agentFeatures: {},
			search: "view=services",
		});
		expect(container.querySelectorAll("tr[data-service]").length).toBe(5);
		// A snapshot from an older agent carries no write-buffering summary: no badge, never a guess.
		const scanner = container.querySelector(
			'tr[data-service="scanner-ingest"]',
		) as HTMLElement;
		expect(scanner.querySelector('[data-badge="writes"]')).toBeNull();
		expect(queryByRole("alert", undefined, container)).toBeNull();
		expect(container.querySelector('[data-kind="error"]')).toBeNull();
		expect(
			fake.api.commands.filter(([, type]) => NEWER_COMMANDS.includes(type)),
		).toEqual([]);
	});

	test("a hub that knows its limits but not their use states the limits alone", async () => {
		const api = fakeDeviceApi();
		api.fail(
			{ method: "GET", path: "devices/usage" },
			new ApiResponseError({
				status: 404,
				code: "NOT_FOUND",
				message: "Device was not found",
			}),
		);
		const { container } = await mountFleet({ api });
		expect(container.querySelector('[data-slots="limits"]')?.textContent).toBe(
			"Up to 100 devices and 10 unused setup packages. This hub doesn't report how many are in use. Hub limits",
		);
		expect(api.sent("GET", "devices/usage").length).toBe(1);
	});
});

describe("FleetScreen at scale", () => {
	test(
		"200 devices: the headline and windows cap their names, the table pages at 50",
		async () => {
			const { container } = await mountFleet({
				seed: generateFleet(200).input,
				unlock: "none",
			});
			const headline = container.querySelector("[data-headline]")?.textContent;
			expect(headline).toMatch(/and \d+ others/);
			const unknown = container.querySelector(
				'[data-window="unknown"]',
			) as HTMLElement;
			expect(unknown.textContent).toMatch(
				/^\d+Status unknown[^,]+, [^,]+, [^,]+ \+\d+ more · locked$/,
			);
			expect(
				container.querySelectorAll("#devices-fleet-list tr[data-device]")
					.length,
			).toBe(50);
			expect(byRole("button", "Show 50 more")).toBeTruthy();
			expect(
				container.querySelectorAll("[data-exception]").length,
			).toBeLessThanOrEqual(5);
			expect(
				container.querySelectorAll("#devices-needs-you li[data-attention]")
					.length,
			).toBe(7);
			expect(primaries(container)).toBe(1);
			expect(allByRole("table", undefined, container).length).toBeGreaterThan(
				0,
			);
		},
		SCALE_TIMEOUT_MS,
	);
});
