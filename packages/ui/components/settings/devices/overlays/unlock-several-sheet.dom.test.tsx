import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { generateFleet } from "../../../../lib/device-management/model/__fixtures__/sample-fleet-200";
import type { KeyState } from "../../../../lib/device-management/workspace/types";
import {
	byRole,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type {
	MountDevicesOptions,
	MountedDevices,
} from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { useOverlayStore } = await import("../workspace/overlay-store");

const { lab, cold, edge, warehouse, studio } = SAMPLE_IDS;

const SUBMIT_BUTTON = /^Unlock \d/;
/** R3: no check code, no gate code, no wire value. */
const MACHINE_WORDS = /\b[GD]\d{1,2}\b|\b[a-z]+_[a-z_]+\b/;
const NO_KEYS_LINE = "Skipped: no keys here";

async function closeOverlay() {
	await act(async () => {
		useOverlayStore.getState().close();
	});
}

afterEach(async () => {
	await closeOverlay();
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

async function openSeveral(options: MountDevicesOptions = {}) {
	const mounted = await mountDevices(<div />, { overlays: true, ...options });
	await act(async () => {
		useOverlayStore.getState().openUnlockSeveral();
	});
	await mounted.settle();
	return mounted;
}

function sheet() {
	return byRole("dialog");
}

function sheetText() {
	return sheet().textContent ?? "";
}

function passwordInput() {
	return sheet().querySelector("input[data-slot=input]") as HTMLInputElement;
}

function trimmed(row: Element) {
	return (row.textContent ?? "").trim();
}

function lines() {
	const list = byRole("list", "Devices to unlock");
	return Array.from(list.querySelectorAll("li"), trimmed);
}

function hasNoKeys(line: string) {
	return line.includes(NO_KEYS_LINE);
}

function lineText(deviceId: string) {
	return sheet().querySelector(`li[data-device="${deviceId}"]`)?.textContent;
}

function resultText() {
	return byRole("status", undefined, sheet()).textContent;
}

function keyState(mounted: MountedDevices, deviceId: string) {
	return mounted.fake.workspace.keys.snapshot(deviceId).state;
}

async function untilKeyState(
	mounted: MountedDevices,
	deviceId: string,
	state: KeyState,
) {
	for (
		let round = 0;
		round < 40 && keyState(mounted, deviceId) !== state;
		round++
	)
		await mounted.settle();
	expect(keyState(mounted, deviceId)).toBe(state);
}

async function untilDone(mounted: MountedDevices) {
	for (let round = 0; round < 40 && !queryByRole("button", "Done"); round++)
		await mounted.settle();
	expect(queryByRole("button", "Done")).not.toBeNull();
}

async function submit(mounted: MountedDevices, password: string) {
	await typeInto(passwordInput(), password);
	await click(byRole("button", SUBMIT_BUTTON, sheet()));
	await mounted.settle();
}

function holdIdentity(mounted: MountedDevices, deviceId: string) {
	return mounted.fake.api.hold({
		path: new RegExp(`^devices/${deviceId}/identity$`),
	});
}

/** The generated fleet with keys for its first 40 devices only. */
function fleetWithFewKeys() {
	const seed = generateFleet(200).input;
	const vaults = seed.local.vaults.slice(0, 40);
	const withKeys = new Set<string>();
	for (const vault of vaults) withKeys.add(vault.deviceId);
	const backups: typeof seed.local.backups = {};
	for (const [deviceId, backup] of Object.entries(seed.local.backups))
		if (withKeys.has(deviceId)) backups[deviceId] = backup;
	seed.local = { ...seed.local, vaults, backups };
	let noKeys = 0;
	for (const row of seed.devices) if (!withKeys.has(row.device_id)) noKeys += 1;
	return { seed, withKeys: withKeys.size, noKeys };
}

describe("unlock several sheet", () => {
	test("lists every device that isn't open: locked ones ticked, the rest skipped with the reason", async () => {
		const mounted = await openSeveral();
		expect(sheetText()).toContain("Unlock several devices");
		expect(sheetText()).toContain(
			"One password, tried on each selected device's keys on this computer.",
		);
		expect(lines()).toEqual([
			"cold-storage-nasOwner keys",
			"old-kioskSkipped: keys for a revoked device",
			"lab-gpu-02Shared-access keys",
			"partner-edgeSkipped: no keys here",
		]);
		const locked = byRole("checkbox", "lab-gpu-02");
		const revoked = byRole("checkbox", "old-kiosk");
		expect(locked.getAttribute("aria-checked")).toBe("true");
		expect(revoked.hasAttribute("disabled")).toBe(true);
		expect(revoked.getAttribute("aria-checked")).toBe("false");
		expect(sheetText()).toContain(
			"Each device takes about a second. Devices where the password fails stay locked.",
		);
		expect(sheet().querySelectorAll("[data-dv-primary]")).toHaveLength(1);
		expect(sheetText()).not.toMatch(MACHINE_WORDS);

		const calls = mounted.fake.api.calls.length;
		const unlock = byRole("button", "Unlock 2 devices");
		expect(unlock.getAttribute("aria-disabled")).toBe("true");
		await click(unlock);
		await mounted.settle();
		expect(mounted.fake.api.calls).toHaveLength(calls);
		expect(keyState(mounted, lab)).toBe("locked");
	});

	test("tries the password on each device in turn and keeps the results listed", async () => {
		const mounted = await openSeveral();
		const release = holdIdentity(mounted, lab);
		await submit(mounted, mounted.fake.password);
		await untilKeyState(mounted, cold, "unlocked");
		await mounted.settle();
		expect(lineText(cold)).toContain("Unlocked");
		expect(lineText(lab)).toContain("Trying…");
		expect(passwordInput().disabled).toBe(true);
		const busy = byRole("button", "Unlock 2 devices");
		expect(busy.getAttribute("aria-busy")).toBe("true");
		release();
		await untilDone(mounted);
		expect(keyState(mounted, lab)).toBe("unlocked");
		expect(lineText(lab)).toContain("Unlocked");
		expect(resultText()).toBe("2 of 2 devices unlocked.");
		expect(sheet().querySelector("input")).toBeNull();
		expect(sheet().innerHTML).not.toContain(mounted.fake.password);
		expect(sheet().querySelectorAll("[data-dv-primary]")).toHaveLength(1);
		await click(byRole("button", "Done"));
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
	});

	test("skips the devices where it fails, names them, and lets another password be tried", async () => {
		const mounted = await openSeveral();
		const opened = mounted.fake.crypto.controllers.length;
		await submit(mounted, "not the password");
		await untilDone(mounted);
		expect(lineText(lab)).toContain("This password didn't open these keys");
		expect(lineText(cold)).toContain("This password didn't open these keys");
		expect(resultText()).toBe(
			"0 of 2 devices unlocked. Still locked: cold-storage-nas and lab-gpu-02.",
		);
		expect(keyState(mounted, lab)).toBe("locked");
		expect(mounted.fake.crypto.controllers).toHaveLength(opened);

		await click(byRole("button", "Try another password"));
		expect(passwordInput().value).toBe("");
		expect(lineText(lab)).toContain("Shared-access keys");
		await submit(mounted, mounted.fake.password);
		await untilDone(mounted);
		expect(resultText()).toBe("2 of 2 devices unlocked.");
		expect(queryByRole("button", "Try another password")).toBeNull();
	});

	test("a device held by another window is skipped, the others still open", async () => {
		const mounted = await openSeveral();
		const release = mounted.fake.holdElsewhere(cold);
		await submit(mounted, mounted.fake.password);
		await untilDone(mounted);
		expect(lineText(lab)).toContain("Unlocked");
		expect(lineText(cold)).toContain("Skipped: unlocked in another window");
		expect(resultText()).toBe(
			"1 of 2 devices unlocked. Still locked: cold-storage-nas.",
		);
		release();
	});

	test("an unticked device is left alone", async () => {
		const mounted = await openSeveral();
		await click(byRole("checkbox", "cold-storage-nas"));
		await typeInto(passwordInput(), mounted.fake.password);
		await click(byRole("button", "Unlock 1 device"));
		await untilDone(mounted);
		expect(keyState(mounted, lab)).toBe("unlocked");
		expect(keyState(mounted, cold)).toBe("locked");
		expect(lineText(cold)).toContain("Owner keys");
		expect(resultText()).toBe("1 of 1 device unlocked.");
	});

	test("nothing left to unlock: says so and leads to Keys & recovery", async () => {
		const mounted = await openSeveral({
			unlock: [edge, warehouse, studio, lab, cold],
		});
		expect(sheetText()).toContain(
			"Every device with keys here is already unlocked.",
		);
		expect(lines()).toEqual([
			"old-kioskSkipped: keys for a revoked device",
			"partner-edgeSkipped: no keys here",
		]);
		expect(passwordInput().disabled).toBe(true);
		const unlock = byRole("button", "Unlock 0 devices");
		expect(unlock.getAttribute("aria-disabled")).toBe("true");
		await click(byRole("button", "Open Keys & recovery"));
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(mounted.navigations.at(-1)?.href).toBe(
			"/settings/devices?view=keys",
		);
	});

	test("closing while it runs stops after the device in hand", async () => {
		const mounted = await openSeveral();
		const release = holdIdentity(mounted, cold);
		await submit(mounted, mounted.fake.password);
		expect(keyState(mounted, cold)).toBe("unlocking");
		await closeOverlay();
		release();
		await untilKeyState(mounted, cold, "locked");
		expect(queryByRole("dialog")).toBeNull();
		expect(keyState(mounted, lab)).toBe("locked");
	});

	test("a 200-device fleet lists what a password can open in a list that scrolls on its own, and caps the rest", async () => {
		const fleet = fleetWithFewKeys();
		expect(fleet.noKeys).toBeGreaterThan(100);
		await openSeveral({ seed: fleet.seed, unlock: "none" });
		const rows = lines();
		expect(rows.filter(hasNoKeys)).toHaveLength(3);
		expect(rows.at(-1)).toBe(
			`${fleet.noKeys - 3} more devices have no keys on this computer.`,
		);
		expect(rows.length).toBeLessThanOrEqual(fleet.withKeys + 4);
		const list = byRole("list", "Devices to unlock");
		expect(list.className).toContain("overflow-y-auto");
	});
});
