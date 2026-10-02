import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { KeyState } from "../../../../lib/device-management/workspace/types";
import {
	byRole,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeCall } from "../testing/fake-device-api";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type {
	MountDevicesOptions,
	MountedDevices,
} from "../testing/mount-devices";
import type { UnlockRequest } from "../workspace/overlay-store";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { useOverlayStore } = await import("../workspace/overlay-store");
const { fakeKeys } = await import("../testing/fake-device-api");
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { AreaOverlays } = await import("./area-overlays");

const { lab, edge, warehouse, cold, partner, oldKiosk } = SAMPLE_IDS;

const BACKUP_ROUTE = /^devices\/controller-vaults\//;
const IDENTITY_ROUTE = /\/identity$/;
const UNLOCK_BUTTON = /^Unlock/;
const LIVE_BOX = /^Connect live now/;
const BACKUP_BOX = /^Also save to your account backup/;
/** R3: no check code, no gate code, no wire value. */
const MACHINE_WORDS = /\b[GD]\d{1,2}\b|\b[a-z]+_[a-z_]+\b/;
const GROUPED_FINGERPRINT = /^[\w-]{4} [\w-]{4} [\w-]{4} [\w-]{4}$/;
const DIRECT_OR_RELAYED = /Direct connection|Connected through the hub/;
const TO_ACTIVITY = {
	screen: "device",
	deviceId: lab,
	tab: "activity",
} as const;

/** Command variants that only newer agents know (plan §3.4.3). */
const NEW_COMMANDS = new Set([
	"host_operation",
	"rollout_history",
	"operations",
	"metrics_history",
	"offline_queue_operations",
	"offline_queue_lookup",
]);

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

async function showUnlock(
	mounted: MountedDevices,
	deviceId: string,
	request: UnlockRequest = {},
) {
	await act(async () => {
		useOverlayStore.getState().openUnlock(deviceId, request);
	});
	await mounted.settle();
}

async function openUnlock(
	deviceId: string,
	options: MountDevicesOptions = {},
	request: UnlockRequest = {},
) {
	const mounted = await mountDevices(<div />, { overlays: true, ...options });
	await showUnlock(mounted, deviceId, request);
	return mounted;
}

interface Row {
	state: string | null;
	text: string;
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

function rowOf(row: Element): Row {
	return { state: row.getAttribute("data-state"), text: row.textContent ?? "" };
}

function listRows(name: string) {
	return Array.from(byRole("list", name).querySelectorAll("li"), rowOf);
}

function checkRows() {
	return listRows("Checks before unlocking");
}

function stateOf(row: Row) {
	return row.state;
}

function textOf(row: Row) {
	return row.text;
}

function checkStates() {
	return checkRows().map(stateOf);
}

function progressTexts() {
	return listRows("Unlock progress").map(textOf);
}

function checkWith(text: string) {
	for (const row of checkRows()) if (row.text.includes(text)) return row;
	throw new Error(`no check says "${text}"`);
}

function alertText() {
	return queryByRole("alert", undefined, sheet())?.textContent ?? null;
}

function keyState(mounted: MountedDevices, deviceId: string) {
	return mounted.fake.workspace.keys.snapshot(deviceId).state;
}

function lastHref(mounted: MountedDevices) {
	return mounted.navigations.at(-1)?.href;
}

function trimmed(cell: Element) {
	return (cell.textContent ?? "").trim();
}

function cellTexts(root: Element, selector: string) {
	return Array.from(root.querySelectorAll(selector), trimmed);
}

function isNewCommand(type: string) {
	return NEW_COMMANDS.has(type);
}

function commandType(command: readonly [string, string, unknown]) {
	return command[1];
}

/** The highest number of times one route was requested: 1 means nothing was retried. */
function mostRepeats(calls: readonly FakeCall[]) {
	const seen = new Map<string, number>();
	for (const [method, path] of calls) {
		const route = `${method} ${path}`;
		seen.set(route, (seen.get(route) ?? 0) + 1);
	}
	return Math.max(0, ...seen.values());
}

/* Unlocking and connecting take a few fake round trips: settle until the sheet shows it. */

async function untilText(mounted: MountedDevices, text: string) {
	for (let round = 0; round < 40 && !sheetText().includes(text); round++)
		await mounted.settle();
	expect(sheetText()).toContain(text);
}

async function untilAlert(mounted: MountedDevices) {
	for (let round = 0; round < 40 && alertText() === null; round++)
		await mounted.settle();
	expect(alertText()).not.toBeNull();
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

async function untilPasswordEnabled(mounted: MountedDevices) {
	for (let round = 0; round < 40 && passwordInput().disabled; round++)
		await mounted.settle();
	expect(passwordInput().disabled).toBe(false);
}

async function submit(mounted: MountedDevices, password: string) {
	await typeInto(passwordInput(), password);
	await click(byRole("button", UNLOCK_BUTTON, sheet()));
	await mounted.settle();
}

function refuseClock(fake: FakeWorkspace, deviceId: string) {
	const row = fake.hub.rows.get(deviceId);
	if (!row) throw new Error(`the sample has no device ${deviceId}`);
	const now = fake.hub.now();
	row.auth_rejection = {
		code: "clock_skew",
		skew_seconds: 420,
		count: 3,
		first_at: now - 900,
		last_at: now - 60,
	};
}

function changeIdentity(fake: FakeWorkspace, deviceId: string) {
	const row = fake.hub.rows.get(deviceId);
	if (!row) throw new Error(`the sample has no device ${deviceId}`);
	row.identity = fakeKeys.identity("someone-else");
}

describe("unlock sheet", () => {
	test("lists the checks by plane, then asks for the device password", async () => {
		const fake = await createFakeWorkspace();
		const opened = fake.crypto.controllers.length;
		const writes = fake.api.writes().length;
		const mounted = await openUnlock(lab, { fake });
		expect(sheetText()).toContain("Unlock lab-gpu-02");
		expect(sheetText()).toContain(
			"One password opens this device's keys on this computer for this window. It never leaves this computer and the hub never sees it.",
		);
		const rows = checkRows();
		expect(checkStates()).toEqual(Array(8).fill("pass"));
		expect(rows[0]?.text).toContain("Hub ready");
		expect(rows[2]?.text).toContain("Shared-access keys on this computer");
		expect(rows[2]?.text).toContain("This computer");
		expect(rows[4]?.text).toContain("Not unlocked in another window");
		expect(rows[5]?.text).toContain("Hub + this computer");
		expect(rows[7]?.text).toContain("Hub + this computer");
		expect(byRole("textbox", "Device password for lab-gpu-02")).toBe(
			passwordInput(),
		);
		expect(passwordInput().disabled).toBe(false);
		expect(sheetText()).toContain(
			"The password never leaves this computer and the hub never sees it.",
		);
		expect(sheet().querySelectorAll("[data-dv-primary]")).toHaveLength(1);
		expect(sheetText()).not.toMatch(MACHINE_WORDS);
		expect(mounted.fake.crypto.controllers).toHaveLength(opened);
		expect(mounted.fake.api.writes()).toHaveLength(writes);
	});

	test("the right password opens the keys, shows each connection step and the result", async () => {
		const mounted = await openUnlock(lab);
		await submit(mounted, mounted.fake.password);
		await untilText(mounted, "Live connection open");
		expect(keyState(mounted, lab)).toBe("unlocked");
		expect(progressTexts()).toEqual([
			"Keys unlocked",
			"Device identity matches",
			"Connection pass received",
			"Device answered",
			expect.stringMatching(DIRECT_OR_RELAYED),
			"Connection secured",
			"Services read",
		]);
		expect(sheetText()).toContain(
			"Unlocked. lab-gpu-02 locks after 30 min unused.",
		);
		expect(sheet().querySelector("input")).toBeNull();
		expect(sheet().innerHTML).not.toContain(mounted.fake.password);
		expect(sheet().querySelectorAll("[data-dv-primary]")).toHaveLength(1);
		await click(byRole("button", "Done"));
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(useOverlayStore.getState().overlay.kind).toBe("none");
	});

	test("without Connect live only the keys open and the encrypted status is read", async () => {
		const mounted = await openUnlock(lab);
		const live = byRole("checkbox", "Connect live now");
		expect(live.getAttribute("aria-checked")).toBe("true");
		await click(live);
		await submit(mounted, mounted.fake.password);
		await untilText(mounted, "Unlocked.");
		expect(progressTexts()).toEqual([
			"Keys unlocked",
			"Device identity matches",
			"Encrypted status read",
		]);
		expect(sheetText()).toContain(
			"Reading encrypted snapshots; no live connection.",
		);
		expect(mounted.fake.workspace.live.state(lab).kind).toBe("idle");
	});

	test("a wrong password says so, empties the field and opens no keys", async () => {
		const mounted = await openUnlock(lab);
		const opened = mounted.fake.crypto.controllers.length;
		await submit(mounted, "not the password");
		await untilAlert(mounted);
		expect(alertText()).toBe(
			"That password doesn't open the keys for this device on this computer.",
		);
		expect(passwordInput().value).toBe("");
		expect(passwordInput().disabled).toBe(false);
		expect(keyState(mounted, lab)).toBe("locked");
		expect(mounted.fake.crypto.controllers).toHaveLength(opened);
		expect(queryByRole("list", "Unlock progress")).toBeNull();
	});

	test("a lock taken elsewhere after the checks clears the password before any key is loaded", async () => {
		const mounted = await openUnlock(lab);
		const { crypto } = mounted.fake;
		const loads = crypto.loads;
		const opened = crypto.controllers.length;
		const release = mounted.fake.holdElsewhere(lab);
		await submit(mounted, mounted.fake.password);
		await untilAlert(mounted);
		expect(passwordInput().value).toBe("");
		expect(alertText()).toBe(
			"This device is unlocked in another tab or window.",
		);
		expect(crypto.controllers).toHaveLength(opened);
		expect(crypto.loads).toBe(loads);
		expect(keyState(mounted, lab)).toBe("held_elsewhere");
		release();
	});

	test("held in another window: the password waits for Use here", async () => {
		const fake = await createFakeWorkspace();
		const release = fake.holdElsewhere(lab);
		const mounted = await openUnlock(lab, { fake });
		const lock = checkWith("Unlocked in another window");
		expect(lock.state).toBe("fail");
		expect(lock.text).toContain(
			"Only one window can hold these keys at a time. The other window locks itself when you take them here.",
		);
		expect(passwordInput().disabled).toBe(true);
		expect(sheetText()).toContain("Fix the failed check above first.");
		await click(byRole("button", "Use here"));
		await untilPasswordEnabled(mounted);
		expect(checkStates()).toEqual(Array(8).fill("pass"));
		release();
	});

	test("a changed identity is a hard block with the comparison on the device", async () => {
		const fake = await createFakeWorkspace(undefined, { unlock: "none" });
		changeIdentity(fake, edge);
		const mounted = await openUnlock(edge, { fake });
		const banner = byRole("alert", undefined, sheet());
		expect(banner.textContent).toContain(
			"The hub reports different keys for edge-berlin-01 than the ones you trusted on Mar 14",
		);
		expect(banner.textContent).toContain(
			"The device may have been set up again, or someone may be impersonating it. Management is blocked.",
		);
		expect(passwordInput().disabled).toBe(true);
		expect(sheet().querySelector("section")).toBeNull();
		const compare = byRole("button", "Compare on the device");
		await click(compare);
		expect(compare.getAttribute("aria-expanded")).toBe("true");
		const panel = sheet().querySelector("section") as HTMLElement;
		expect(panel.id).toBe(compare.getAttribute("aria-controls") ?? "");
		expect(panel.textContent).toContain("flow-like-standalone status");
		expect(panel.textContent).toContain("Trusted on this computer");
		expect(panel.textContent).toContain("Reported by the hub now");
		expect(panel.textContent).toContain(
			"An agent that prints no fingerprint is too old for this check.",
		);
		const prints = cellTexts(panel, "dd");
		expect(prints).toHaveLength(2);
		expect(prints[0]).toMatch(GROUPED_FINGERPRINT);
		expect(prints[1]).toMatch(GROUPED_FINGERPRINT);
		expect(prints[0]).not.toBe(prints[1]);
		await submit(mounted, fake.password);
		expect(keyState(mounted, edge)).toBe("blocked");
		expect(fake.crypto.controllers).toHaveLength(0);
		await click(byRole("button", "Review identity"));
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(lastHref(mounted)).toBe(`/settings/devices?device=${edge}&tab=keys`);
	});

	test("names the clock-skew refusal the hub recorded", async () => {
		const fake = await createFakeWorkspace();
		refuseClock(fake, lab);
		await openUnlock(lab, { fake });
		const clocks = checkRows().at(-1);
		expect(clocks?.state).toBe("warn");
		expect(clocks?.text).toContain("lab-gpu-02's clock looks 7 min off");
		expect(clocks?.text).toContain(
			"The hub refused 3 check-ins from lab-gpu-02 since",
		);
		expect(clocks?.text).toContain(
			"Its clock is about 7 min ahead of the hub's.",
		);
		expect(passwordInput().disabled).toBe(false);
	});

	test("an older hub records no refusal: the skew is the estimate, nothing errors and nothing is retried", async () => {
		const fake = await createFakeWorkspace(undefined, { hubVersion: "old" });
		fake.workspace.clock.observe(
			"snapshot",
			fake.hub.now() + 420,
			fake.clock.now(),
			lab,
		);
		const mounted = await mountDevices(<div />, { overlays: true, fake });
		const before = fake.api.calls.length;
		await showUnlock(mounted, lab);
		const clocks = checkRows().at(-1);
		expect(clocks?.state).toBe("warn");
		expect(clocks?.text).toContain("lab-gpu-02's clock looks 7 min off");
		expect(clocks?.text).toContain(
			"Estimated from the device's last encrypted status. Turn on automatic time sync on the device.",
		);
		expect(clocks?.text).not.toContain("The hub refused");
		expect(alertText()).toBeNull();
		expect(passwordInput().disabled).toBe(false);
		expect(mostRepeats(fake.api.calls.slice(before))).toBeLessThanOrEqual(1);
	});

	test("a deep link to a locked section lands on the requested tab after the unlock", async () => {
		const mounted = await openUnlock(lab, {}, { returnTo: TO_ACTIVITY });
		expect(mounted.navigations).toEqual([]);
		await submit(mounted, mounted.fake.password);
		await untilKeyState(mounted, lab, "unlocked");
		await mounted.settle();
		expect(lastHref(mounted)).toBe(
			`/settings/devices?device=${lab}&tab=activity`,
		);
		expect(queryByRole("dialog")).not.toBeNull();
	});

	test("an offline device: Connect live unticks itself and says why", async () => {
		await openUnlock(warehouse, { unlock: "none" });
		const checkIn = checkWith("Offline since");
		expect(checkIn.state).toBe("warn");
		expect(checkIn.text).toContain(
			"A live connection will likely fail; encrypted snapshots still work.",
		);
		const live = byRole("checkbox", LIVE_BOX);
		expect(live.getAttribute("aria-checked")).toBe("false");
		expect(live.hasAttribute("disabled")).toBe(true);
		expect(sheetText()).toContain(
			"(warehouse-pi is offline, so only encrypted snapshots can be read)",
		);
		expect(passwordInput().disabled).toBe(false);
		expect(byRole("button", "Diagnose", sheet())).toBeTruthy();
	});

	test("no keys here or a revoked device: the password stays disabled and Unlock sends nothing", async () => {
		const mounted = await openUnlock(partner);
		expect(checkWith("No keys here").text).toContain(
			"No keys here for this account, hub and profile. They may be in another browser, app profile or hub.",
		);
		expect(checkWith("No keys here").state).toBe("fail");
		expect(passwordInput().disabled).toBe(true);
		const calls = mounted.fake.api.calls.length;
		const unlock = byRole("button", "Unlock", sheet());
		expect(unlock.getAttribute("aria-disabled")).toBe("true");
		await click(unlock);
		await mounted.settle();
		expect(mounted.fake.api.calls).toHaveLength(calls);
		expect(alertText()).toBeNull();
		await click(byRole("button", "Restore keys…"));
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(lastHref(mounted)).toBe(
			`/settings/devices?view=keys&focus=${partner}`,
		);

		await showUnlock(mounted, oldKiosk);
		expect(checkWith("Device revoked").state).toBe("fail");
		expect(passwordInput().disabled).toBe(true);
	});

	test("Forgot it? opens the guide in Keys & recovery", async () => {
		const mounted = await openUnlock(lab);
		await click(byRole("button", "Forgot it?"));
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(lastHref(mounted)).toBe(
			"/settings/devices?view=keys&guide=forgot-password",
		);
	});

	test("the same password is tried on the other locked devices, and the results stay listed", async () => {
		const mounted = await openUnlock(lab);
		await click(byRole("checkbox", "Unlock other devices with this password"));
		const others = byRole("list", "Other devices");
		expect(others.textContent).toContain("cold-storage-nas");
		expect(others.textContent).toContain("Owner keys");
		expect(others.textContent).not.toContain("old-kiosk");
		expect(byRole("button", "Unlock 2 devices")).toBeTruthy();
		await submit(mounted, mounted.fake.password);
		await untilKeyState(mounted, cold, "unlocked");
		await mounted.settle();
		expect(keyState(mounted, lab)).toBe("unlocked");
		expect(byRole("list", "Other devices").textContent).toContain("Unlocked");
		expect(sheet().innerHTML).not.toContain(mounted.fake.password);
	});

	test("offers the account backup only while the device's backup needs it", async () => {
		const mounted = await openUnlock(cold);
		const backup = byRole("checkbox", BACKUP_BOX);
		expect(backup.getAttribute("aria-checked")).toBe("false");
		await click(backup);
		await submit(mounted, mounted.fake.password);
		await untilText(mounted, "Unlocked.");
		expect(progressTexts()).toContain("Saved to your account backup");
		expect(mounted.fake.api.sent("PUT", BACKUP_ROUTE)).toHaveLength(1);

		await showUnlock(mounted, lab);
		expect(queryByRole("checkbox", BACKUP_BOX)).toBeNull();
		await submit(mounted, mounted.fake.password);
		await untilKeyState(mounted, lab, "unlocked");
		expect(mounted.fake.api.sent("PUT", BACKUP_ROUTE)).toHaveLength(1);
	});

	test("an older agent connects with the commands it knows", async () => {
		const mounted = await openUnlock(lab, { agentFeatures: {} });
		await submit(mounted, mounted.fake.password);
		await untilText(mounted, "Live connection open");
		expect(alertText()).toBeNull();
		const sent = mounted.fake.api.commands.map(commandType);
		expect(sent.length).toBeGreaterThan(0);
		expect(sent.filter(isNewCommand)).toEqual([]);
	});

	test("closing while it unlocks drops the late completion", async () => {
		const mounted = await openUnlock(lab, {}, { returnTo: TO_ACTIVITY });
		const release = mounted.fake.api.hold({ path: IDENTITY_ROUTE });
		await typeInto(passwordInput(), mounted.fake.password);
		await click(byRole("button", "Unlock", sheet()));
		expect(keyState(mounted, lab)).toBe("unlocking");
		await closeOverlay();
		release();
		await untilKeyState(mounted, lab, "locked");
		expect(queryByRole("dialog")).toBeNull();
		expect(mounted.navigations).toEqual([]);
	});

	test("works under the workspace provider alone, as on the Events page", async () => {
		const hrefs: string[] = [];
		const mounted = await mountDevices(
			<AreaOverlays
				onNavigate={(href) => {
					hrefs.push(href);
				}}
			/>,
			{ passive: true },
		);
		await showUnlock(mounted, lab);
		await submit(mounted, mounted.fake.password);
		await untilKeyState(mounted, lab, "unlocked");
		await mounted.settle();
		expect(sheetText()).toContain("Unlocked.");
		await click(byRole("button", "Done"));
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();

		await showUnlock(mounted, cold);
		await click(byRole("button", "Forgot it?"));
		await mounted.settle();
		expect(hrefs).toEqual([
			"/settings/devices?view=keys&guide=forgot-password",
		]);
		expect(mounted.navigations).toEqual([]);
	});

	test("the request is dropped when the host unmounts", async () => {
		const mounted = await openUnlock(lab);
		await mounted.unmount();
		await act(async () => {
			await Promise.resolve();
		});
		expect(useOverlayStore.getState().overlay.kind).toBe("none");
	});

	test("Copy diagnostics copies a plain report with the check codes and no secrets", async () => {
		const mounted = await openUnlock(lab);
		await typeInto(passwordInput(), mounted.fake.password);
		await click(byRole("button", "Copy diagnostics"));
		await mounted.settle();
		const report = dom.clipboard.at(-1) ?? "";
		const lines = report.split("\n");
		expect(report).toContain("Flow-Like device pre-flight");
		expect(lines).toContain("D1 pass hub_ready");
		expect(lines).toContain("key session: locked");
		expect(lines.some(startsWithD9)).toBe(true);
		expect(report).not.toContain(mounted.fake.password);
		expect(byRole("button", "Diagnostics copied")).toBeTruthy();
	});
});

function startsWithD9(line: string) {
	return line.startsWith("D9 ");
}
