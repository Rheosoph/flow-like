import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import {
	SAMPLE_APPS,
	SAMPLE_IDS,
	SAMPLE_NOW,
	sampleFleet,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { DeviceRoute } from "../../../../lib/device-management/model/types";
import {
	byRole,
	byText,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { readDeviceVault } = await import(
	"../../../../lib/device-management/storage"
);
const { fakeDeviceApi } = await import("../testing/fake-device-api");
const { ApiResponseError } = await import("../../../../lib/api-error");
const { DeviceKeysTab } = await import("./keys-tab");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const ACCOUNT = { kind: "account" } as const;
const MACHINE_WORDS =
	/[a-z]+_[a-z_]+|\bG\d{1,2}\b|\bD\d{1,2}\b|controller|vault|MLS|endpoint/;

function mount(deviceId: string, options: MountDevicesOptions = {}) {
	const route: DeviceRoute = { screen: "device", deviceId, tab: "keys" };
	return mountDevices(
		<DeviceKeysTab route={route} scope={ACCOUNT} deviceId={deviceId} />,
		options,
	);
}

type View = Awaited<ReturnType<typeof mount>>;

function keyWrites(view: View) {
	const ours = ([, path]: [string, string, unknown?]) =>
		path.includes("controller-vaults");
	return view.fake.api.writes().filter(ours);
}

function linkOf(link: Element) {
	return [
		link.querySelector("b")?.firstChild?.textContent,
		link.getAttribute("data-state"),
	];
}

function chain(container: HTMLElement) {
	const links = container.querySelectorAll("ol[aria-label] > li[data-state]");
	return Array.from(links).map(linkOf);
}

function withoutKeys(): MountDevicesOptions {
	const seed = sampleFleet();
	seed.local = { ...seed.local, vaults: [], backups: {} };
	seed.keys = [];
	return { seed, unlock: "none" };
}

async function endpointId(view: View): Promise<string | undefined> {
	const stored = await readDeviceVault(view.fake.scope, SAMPLE_IDS.edge);
	return stored?.controllerPublic.endpoint_id;
}

/** Opens Advanced › Reset metric-group identity on a device with two metric groups. */
async function openReset(view: View): Promise<HTMLElement> {
	await act(async () => {
		view.fake.workspace.facts.record(SAMPLE_IDS.edge, {
			metricReaders: [
				{ scope: "device", expiresAt: SAMPLE_NOW + 86_400 },
				{ scope: SAMPLE_APPS.supportPortal, expiresAt: SAMPLE_NOW + 86_400 },
			],
		});
	});
	await click(byText("Advanced", view.container));
	await click(byRole("button", "Reset metric-group identity…", view.container));
	await view.settle();
	return byRole("alertdialog");
}

async function resetWith(view: View, password: string): Promise<void> {
	const sheet = byRole("alertdialog");
	await typeInto(sheet.querySelector("input") as HTMLInputElement, password);
	await click(byRole("button", "Reset identity", sheet));
	await view.settle();
}

describe("device Keys tab: keys on this computer", () => {
	test("shows the trust chain, the backup versions and every key action for an unlocked owner device", async () => {
		const view = await mount(SAMPLE_IDS.edge);
		const { container } = view;
		expect(chain(container)).toEqual([
			["Keys on this computer", "good"],
			["Device identity", "good"],
			["Access rules v5", "good"],
			["Certificates", "warning"],
		]);
		expect(container.textContent).toContain(
			"Owner keys · unlocked · backed up to your account (v3)",
		);
		expect(container.textContent).toContain(
			"enforces access rules v5, signed with your owner key",
		);
		expect(
			container
				.querySelector('[data-dvo="versions"]')
				?.getAttribute("aria-label"),
		).toBe("This computer has version 3; your account has version 3");
		expect(container.textContent).toContain("In sync");
		expect(container.textContent).toContain("Last saved");
		expect(container.textContent).toContain("never downloaded here");
		expect(container.textContent).toContain("Desktop app · kept safely");
		for (const name of [
			"Update account backup",
			"Check account backup",
			"Download key backup file…",
			"Change device password…",
			"Delete keys from this computer…",
		])
			expect(
				byRole("button", name, container).getAttribute("aria-disabled"),
			).toBeNull();
		expect(byText("Forgot the device password?", container)).toBeTruthy();
		expect(container.querySelectorAll("[data-dv-primary]").length).toBe(0);
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
		expect(keyWrites(view)).toEqual([]);
	});

	test("a locked device keeps Download visible and disabled with the reason; clicking sends nothing", async () => {
		const view = await mount(SAMPLE_IDS.cold);
		const { container } = view;
		const download = byRole("button", "Download key backup file…", container);
		expect(download.getAttribute("aria-disabled")).toBe("true");
		expect(container.textContent).toContain(
			"Unlock cold-storage-nas to use its keys.",
		);
		const calls = view.fake.api.calls.length;
		await click(download);
		expect(queryByRole("dialog")).toBeNull();
		expect(view.fake.api.calls.length).toBe(calls);
		expect(container.textContent).toContain(
			"Not backed up. The keys exist only here.",
		);
		expect(byRole("button", "Back up to account", container)).toBeTruthy();
	});

	test("a shared device names the app its access covers and links to that app's devices", async () => {
		const view = await mount(SAMPLE_IDS.lab);
		const { container } = view;
		expect(container.textContent).toContain("Shared-access keys");
		expect(container.textContent).toContain("Your access");
		const app = byRole("link", "App Invoice AI", container);
		expect(app.getAttribute("href")).toContain(
			`/library/config/devices?id=${SAMPLE_APPS.invoiceAi}`,
		);
		expect(container.textContent).toContain(
			"enforces access rules signed with the owner's key",
		);
	});
});

describe("device Keys tab: other states", () => {
	test("without keys here it offers restore, import and (for shared devices) a new request", async () => {
		const view = await mount(SAMPLE_IDS.lab, withoutKeys());
		const { container } = view;
		expect(container.textContent).toContain(
			"This computer has no keys for lab-gpu-02.",
		);
		expect(container.textContent).toContain(
			"Your account holds a backup (v1); restore it with the device password.",
		);
		expect(
			byRole("button", "Restore from account backup…", container),
		).toBeTruthy();
		expect(byRole("button", "Import backup file…", container)).toBeTruthy();
		expect(byRole("link", "Request access", container)).toBeTruthy();
		expect(chain(container)[0]).toEqual(["Keys on this computer", "unknown"]);

		await click(byRole("button", "Restore from account backup…", container));
		const sheet = byRole("dialog");
		const row = sheet.querySelector(
			`[data-restore-row="${SAMPLE_IDS.lab}"]`,
		) as HTMLElement;
		expect(
			row.querySelector('[role="checkbox"]')?.getAttribute("aria-checked"),
		).toBe("true");
		expect(sheet.querySelectorAll('input[type="password"]').length).toBe(1);
	});

	test("a revoked device only offers to delete the keys that are left here", async () => {
		const view = await mount(SAMPLE_IDS.oldKiosk);
		const { container } = view;
		expect(container.textContent).toContain(
			"Owner keys · for a revoked device",
		);
		expect(container.querySelectorAll('[data-slot="dv-button"]').length).toBe(
			1,
		);
		await click(byRole("button", "Delete keys from this computer…", container));
		const sheet = byRole("alertdialog");
		expect(sheet.textContent).toContain("These keys can't be used any more.");
		await click(byRole("button", "Delete keys for old-kiosk", sheet));
		await view.settle();
		expect(container.textContent).toContain(
			"This computer holds no keys for old-kiosk.",
		);
		expect(container.textContent).toContain(
			"Keys for old-kiosk were deleted from this computer",
		);
	});

	test("a device you only approve cloud access for has no keys to hold", async () => {
		const seed = sampleFleet();
		const partner = seed.devices.find(
			(row) => row.device_id === SAMPLE_IDS.partner,
		);
		if (partner) partner.status = "active";
		const view = await mount(SAMPLE_IDS.partner, { seed });
		expect(view.container.textContent).toContain("No keys to hold");
		expect(queryByRole("button", /Restore/)).toBeNull();
	});

	test("access the hub refuses reads as ended, not as an error", async () => {
		const api = fakeDeviceApi({ seed: sampleFleet() });
		api.fail(
			{ method: "GET", path: `devices/${SAMPLE_IDS.lab}/management/my-access` },
			new ApiResponseError({ status: 403, code: "FORBIDDEN", message: "No" }),
		);
		const { container } = await mount(SAMPLE_IDS.lab, { api });
		expect(chain(container)[2]).toEqual(["Your access", "warning"]);
		expect(container.textContent).toContain("No active permissions");
		expect(container.querySelector('[role="alert"]')).toBeNull();
	});
});

describe("device Keys tab: older hub and older agent", () => {
	test("an older hub shows the backup version without a save date, reading each route once", async () => {
		const view = await mount(SAMPLE_IDS.edge, { hubVersion: "old" });
		const { container } = view;
		expect(container.textContent).toContain("In sync");
		expect(container.textContent).toContain(
			"This hub reports the version only, not when it was saved.",
		);
		expect(container.textContent).not.toContain("Last saved");
		expect(container.querySelector('[role="alert"]')).toBeNull();
		expect(
			view.fake.api.sent("GET", "devices/controller-vaults").length,
		).toBeLessThanOrEqual(1);
		expect(
			view.fake.api.sent("GET", `devices/controller-vaults/${SAMPLE_IDS.edge}`)
				.length,
		).toBeLessThanOrEqual(1);
	});

	test("an older hub offers unlock as the way to see a shared device's permissions", async () => {
		const shared = await mount(SAMPLE_IDS.lab, { hubVersion: "old" });
		expect(shared.container.textContent).toContain(
			"Unlock to check your permissions",
		);
		expect(shared.container.querySelector('[role="alert"]')).toBeNull();
		expect(
			shared.fake.api.sent(
				"GET",
				`devices/${SAMPLE_IDS.lab}/management/my-access`,
			).length,
		).toBeLessThanOrEqual(1);
	});

	test("Reset metric-group identity lists the affected groups before it can be confirmed", async () => {
		const view = await mount(SAMPLE_IDS.edge, { agentFeatures: {} });
		const before = await endpointId(view);
		const sheet = await openReset(view);
		expect(sheet.textContent).toContain(
			"2 groups need their owner's approval again",
		);
		const groups = sheet.querySelector("[data-affected-groups]");
		expect(groups?.textContent).toContain("Whole device");
		expect(byRole("link", "App Support Portal", sheet)).toBeTruthy();
		expect(sheet.textContent).toContain("No, this is permanent.");
		const reset = byRole("button", "Reset identity", sheet);
		expect(reset.getAttribute("aria-disabled")).toBe("true");

		await resetWith(view, "this is not the password");
		expect(byRole("alertdialog").textContent).toContain(
			"That password doesn't open the keys for edge-berlin-01 on this computer.",
		);
		expect(await endpointId(view)).toBe(before);
	});

	test("Reset metric-group identity takes the device password and sends no device command", async () => {
		const view = await mount(SAMPLE_IDS.edge, { agentFeatures: {} });
		const { container, fake } = view;
		const before = await endpointId(view);
		const commands = fake.api.commands.length;
		await openReset(view);
		await resetWith(view, fake.password);
		expect(queryByRole("alertdialog")).toBeNull();
		expect(await endpointId(view)).not.toBe(before);
		expect(fake.workspace.keys.snapshot(SAMPLE_IDS.edge).state).toBe("locked");
		expect(container.textContent).toContain(
			"This computer has a fresh identity for shared live metrics on edge-berlin-01",
		);
		expect(fake.api.commands.length).toBe(commands);
	});
});
