import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";
import type { FakeCall, FakeCommand } from "../testing/fake-device-api";
import type { FakeWorkspace } from "../testing/fake-workspace";
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
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { fakeKeys } = await import("../testing/fake-device-api");
const { AreaOverlays } = await import("./area-overlays");

const { lab, cold, edge, warehouse } = SAMPLE_IDS;

/** R3: no check code, no gate code, no wire value. */
const MACHINE_WORDS = /\b[GD]\d{1,2}\b|\b[a-z]+_[a-z_]+\b/;
const OFFLINE_SUB = /Offline since .+ · last check-in/;
const SUPPORT_APP = { kind: "app", appId: "app_support_portal" } as const;
const COMMANDS = [
	"flow-like-standalone status",
	"flow-like-standalone service-status",
	"flow-like-standalone recover-enrollment",
	"./flow-like-standalone install-service",
];

afterEach(async () => {
	await act(async () => {
		useOverlayStore.getState().close();
	});
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

async function showDiagnose(
	mounted: MountedDevices,
	deviceId: string,
	serviceId?: string,
) {
	await act(async () => {
		await mounted.fake.queryClient.invalidateQueries();
		useOverlayStore.getState().openDiagnose(deviceId, serviceId);
	});
	await mounted.settle();
}

async function openDiagnose(
	deviceId: string,
	serviceId?: string,
	options: MountDevicesOptions = {},
) {
	const mounted = await mountDevices(<div />, { overlays: true, ...options });
	await showDiagnose(mounted, deviceId, serviceId);
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

function rowOf(row: Element): Row {
	return { state: row.getAttribute("data-state"), text: row.textContent ?? "" };
}

function sourceRows() {
	const list = byRole("list", "What each source says");
	return Array.from(list.querySelectorAll("li"), rowOf);
}

function stateOf(row: Row) {
	return row.state;
}

function textOf(row: Row) {
	return row.text;
}

function sourceRow(text: string) {
	for (const row of sourceRows()) if (row.text.includes(text)) return row;
	return undefined;
}

function lastHref(mounted: MountedDevices) {
	return mounted.navigations.at(-1)?.href;
}

function commandsTo(commands: readonly FakeCommand[], deviceId: string) {
	let count = 0;
	for (const [target] of commands) if (target === deviceId) count += 1;
	return count;
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

function refuse(
	fake: FakeWorkspace,
	deviceId: string,
	code: "clock_skew" | "revoked_credential",
	skew: number | null,
) {
	const row = fake.hub.rows.get(deviceId);
	if (!row) throw new Error(`the sample has no device ${deviceId}`);
	const now = fake.hub.now();
	row.auth_rejection = {
		code,
		skew_seconds: skew,
		count: 12,
		first_at: now - 7_200,
		last_at: now - 30,
	};
}

describe("diagnose sheet", () => {
	test("an offline device: what each source says, what the hub knows, what to run on it", async () => {
		const mounted = await openDiagnose(warehouse);
		const text = sheetText();
		expect(text).toContain("Diagnose warehouse-pi");
		expect(text).toMatch(OFFLINE_SUB);
		const rows = sourceRows();
		expect(rows.map(stateOf)).toEqual([
			"pass",
			"warn",
			"fail",
			"pass",
			"pending",
		]);
		const [hub, status, live, keys, onDevice] = rows.map(textOf);
		expect(hub).toContain("The hub knows warehouse-pi. Last check-in");
		expect(hub).toContain("Hub");
		expect(status).toContain("Encrypted status from");
		expect(status).toContain("last known");
		expect(status).toContain("Encrypted snapshot");
		expect(live).toContain(
			"A live connection won't work: the device is offline. It needs a check-in within the last 2 minutes.",
		);
		expect(live).toContain("Hub");
		expect(keys).toContain("Owner keys here · Unlocked");
		expect(keys).toContain("This computer");
		expect(onDevice).toContain(
			"The connection state, last contact and each service's last error are only on the device.",
		);
		expect(onDevice).toContain("On-device only");

		expect(text).toContain("What the hub knows");
		expect(text).toContain("Registered");
		expect(text).toContain("0.9.2");
		for (const command of COMMANDS) expect(text).toContain(command);
		expect(text).toContain("Common causes");
		expect(text).toContain("Power or network was lost.");
		expect(text).toContain("Outbound HTTPS to hub.test is blocked.");
		expect(text).not.toMatch(MACHINE_WORDS);
		expect(sheet().querySelectorAll("[data-dv-primary]")).toHaveLength(0);
		expect(queryByRole("alert", undefined, sheet())).toBeNull();
		expect(commandsTo(mounted.fake.api.commands, warehouse)).toBe(0);
	});

	test("a device that never checked in gets the causes of a first check-in that never came", async () => {
		await openDiagnose(cold);
		const text = sheetText();
		expect(text).toContain("never checked in");
		const rows = sourceRows();
		expect(rows[0]?.text).toContain(
			"The hub knows cold-storage-nas: registered",
		);
		expect(rows[1]?.text).toContain(
			"No encrypted status yet: the device sends its first one after its first check-in.",
		);
		expect(rows[2]?.text).toContain(
			"A live connection won't work: the device has never checked in.",
		);
		expect(text).toContain("Never");
		expect(text).toContain(
			"The agent was never started, or it stopped after the setup window closed.",
		);
		expect(text).toContain(
			"Outbound HTTPS to hub.test is blocked by a firewall or proxy.",
		);
	});

	test("a service crashing last known on an offline device leads the sheet", async () => {
		const mounted = await openDiagnose(warehouse, "scanner-ingest");
		const text = sheetText();
		const service = text.indexOf("Last known: scanner-ingest");
		expect(service).toBeGreaterThanOrEqual(0);
		expect(service).toBeLessThan(text.indexOf("What each source says"));
		expect(sheet().querySelector("[data-conv=crash_looping]")).not.toBeNull();
		expect(text).toContain("0 of 1 instances ready");
		expect(text).toContain(
			"The reason for the crash is only on the device. Run flow-like-standalone status there to see it.",
		);
		expect(mounted.navigations).toEqual([]);
	});

	test("a crashing service on a reachable, unlocked device opens its logs with errors only", async () => {
		const mounted = await openDiagnose(edge, "support-bot");
		expect(queryByRole("dialog")).toBeNull();
		expect(lastHref(mounted)).toBe(
			`/settings/devices?device=${edge}&service=support-bot&tab=activity&stream=errors`,
		);
		expect(useOverlayStore.getState().overlay.kind).toBe("none");
	});

	test("names the refusal the hub recorded: a clock that is off, or a credential it no longer accepts", async () => {
		const fake = await createFakeWorkspace();
		refuse(fake, warehouse, "clock_skew", -540);
		const mounted = await openDiagnose(warehouse, undefined, { fake });
		const refused = sourceRows()[1];
		expect(refused?.state).toBe("fail");
		expect(refused?.text).toContain(
			"The hub refused 12 check-ins from warehouse-pi since",
		);
		expect(refused?.text).toContain(
			"Its clock is about 9 min behind the hub's.",
		);
		expect(refused?.text).toContain(
			"Turn on automatic time sync on the device.",
		);

		refuse(fake, cold, "revoked_credential", null);
		await showDiagnose(mounted, cold);
		const credential = sourceRows()[1];
		expect(credential?.text).toContain(
			"The device signs in with a credential the hub no longer accepts.",
		);
		expect(credential?.text).toContain("Run recover-enrollment on the device");
	});

	test("an older hub records no refusal: the estimate from the last encrypted status stands in", async () => {
		const fake = await createFakeWorkspace(undefined, { hubVersion: "old" });
		fake.workspace.clock.observe(
			"snapshot",
			fake.hub.now() - 480,
			fake.clock.now(),
			warehouse,
		);
		const mounted = await mountDevices(<div />, { overlays: true, fake });
		const before = fake.api.calls.length;
		await act(async () => {
			useOverlayStore.getState().openDiagnose(warehouse);
		});
		await mounted.settle();
		expect(sourceRow("The hub refused")).toBeUndefined();
		const estimate = sourceRow("clock looks");
		expect(estimate?.state).toBe("warn");
		expect(estimate?.text).toContain(
			"warehouse-pi's clock looks 8 min off, estimated from its last encrypted status.",
		);
		expect(queryByRole("alert", undefined, sheet())).toBeNull();
		expect(mostRepeats(fake.api.calls.slice(before))).toBeLessThanOrEqual(1);
	});

	test("keys closed for a changed identity say so, not that the browser is at fault", async () => {
		const fake = await createFakeWorkspace(undefined, { unlock: "none" });
		const row = fake.hub.rows.get(edge);
		if (!row) throw new Error("the sample has no edge device");
		row.identity = fakeKeys.identity("someone-else");
		const mounted = await mountDevices(<div />, { overlays: true, fake });
		await act(async () => {
			await fake.workspace.keys.preflight(edge);
		});
		await showDiagnose(mounted, edge);
		const keys = sourceRow("Owner keys here");
		expect(keys?.state).toBe("fail");
		expect(keys?.text).toContain(
			"closed: the hub reports other keys for this device than the ones trusted here.",
		);
		expect(sheetText()).not.toContain("Browser can't protect keys");
		expect(sheetText()).not.toMatch(MACHINE_WORDS);
	});

	test("a locked device offers Unlock, which replaces the sheet", async () => {
		const mounted = await openDiagnose(lab);
		expect(sourceRow("Locked:")?.text).toContain(
			"Locked: unlock to read the encrypted status.",
		);
		expect(sheetText()).toContain("A live connection can be tried.");
		expect(sheetText()).not.toContain("Common causes");
		await click(byRole("button", "Unlock…"));
		await mounted.settle();
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: lab,
		});
		expect(sheetText()).toContain("Unlock lab-gpu-02");
	});

	test("Open goes to the device and Copy diagnostics copies a plain report", async () => {
		const mounted = await openDiagnose(warehouse, "scanner-ingest");
		await click(byRole("button", "Copy diagnostics"));
		await mounted.settle();
		const report = dom.clipboard.at(-1) ?? "";
		expect(report).toContain(`Device: warehouse-pi (${warehouse})`);
		expect(report).toContain("Presence: offline");
		expect(report).toContain("Keys here: unlocked");
		expect(report).toContain(
			"Service scanner-ingest: requested running, actual backoff",
		);
		const open = byRole("link", "Open warehouse-pi");
		const href = `/settings/devices?device=${warehouse}&tab=overview`;
		expect(open.getAttribute("href")).toBe(href);
		await click(open);
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(lastHref(mounted)).toBe(href);
	});

	test("a device the hub doesn't list says so instead of guessing", async () => {
		await openDiagnose("00000000-0000-4000-8000-000000000000");
		expect(sheetText()).toContain(
			"This device isn't in your device list on this hub",
		);
		expect(queryByRole("list", "What each source says")).toBeNull();
	});

	test("works under the workspace provider alone, as on the Events page", async () => {
		const hrefs: string[] = [];
		const mounted = await mountDevices(
			<AreaOverlays
				scope={SUPPORT_APP}
				onNavigate={(href) => {
					hrefs.push(href);
				}}
			/>,
			{ passive: true },
		);
		await act(async () => {
			useOverlayStore.getState().openDiagnose(warehouse);
		});
		await mounted.settle();
		expect(sheetText()).toContain("Diagnose warehouse-pi");
		await click(byRole("link", "Open warehouse-pi"));
		await mounted.settle();
		expect(hrefs).toEqual([
			`/library/config/devices?id=app_support_portal&device=${warehouse}&tab=overview`,
		]);
		expect(queryByRole("dialog")).toBeNull();
	});
});
