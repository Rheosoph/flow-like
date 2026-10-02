import { describe, expect, test } from "bun:test";
import {
	APPS,
	PLACEMENTS,
	sampleDevices,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import { buildAppView } from "../../../../lib/device-management/model/app-plan";
import { runsOnDeviceNames, runsOnRows } from "./runs-on-model";

type AppId = keyof typeof APPS;

function viewOf(appId: AppId, options: { labUnlocked?: boolean } = {}) {
	const devices = sampleDevices(options);
	return buildAppView({
		app: APPS[appId],
		devices,
		placements: PLACEMENTS[appId],
		focusDeviceIds: devices.map((device) => device.id),
	});
}

const rowOf = (appId: AppId, eventId: string, labUnlocked = false) => {
	const row = runsOnRows(viewOf(appId, { labUnlocked })).get(eventId);
	if (!row) throw new Error(`no row for ${eventId}`);
	return row;
};

const devicesOf = (list: readonly { deviceId: string }[]) =>
	list.map((entry) => entry.deviceId);

describe("runsOnRows", () => {
	test("splits an event's devices into serving, unknown and elsewhere", () => {
		const row = rowOf("app_invoice_ai", "evt_extract_http");
		expect(devicesOf(row.served)).toEqual(["edge-berlin-01"]);
		expect(row.served[0].service?.serviceId).toBe("invoice-extractor");
		expect(row.served[0].cell.conv).toBe("update_in_progress");
		expect(devicesOf(row.unknown)).toEqual(["lab-gpu-02"]);
		expect(row.unknown[0].unknown.kind).toBe("locked");
		expect(row.lockedOnly).toBe(true);
		expect(devicesOf(row.elsewhere)).toEqual([
			"studio-mac-mini",
			"warehouse-pi",
			"cold-storage-nas",
		]);
	});

	test("counts the devices that serve an older pin than the event's newest", () => {
		const row = rowOf("app_invoice_ai", "evt_extract_http");
		expect(row.older).toBe(1);
		expect(row.pin?.eventVersion).toEqual([1, 5, 0]);
		expect(row.served[0].cell.pin?.eventVersion).toEqual([1, 4, 0]);
		expect(rowOf("app_invoice_ai", "evt_gpu_extract").older).toBe(0);
	});

	test("an unlocked shared device moves from unknown to serving", () => {
		const row = rowOf("app_invoice_ai", "evt_gpu_extract", true);
		expect(devicesOf(row.served)).toEqual(["lab-gpu-02"]);
		expect(row.served[0].service?.serviceId).toBe("invoice-extractor-gpu");
		expect(row.unknown).toEqual([]);
		expect(row.older).toBe(1);
	});

	test("a device that never checked in is never unknown: nothing can run there", () => {
		for (const row of runsOnRows(viewOf("app_invoice_ai")).values()) {
			expect(devicesOf(row.unknown)).not.toContain("cold-storage-nas");
			expect(
				row.elsewhere.find((entry) => entry.deviceId === "cold-storage-nas")
					?.state,
			).toBe("not_served");
		}
	});

	test("a status without an event list is unknown and names the service it does list", () => {
		const row = rowOf("app_warehouse_scan", "evt_scan_station");
		expect(row.served).toEqual([]);
		expect(row.lockedOnly).toBe(false);
		expect(row.unknown.map((entry) => entry.unknown.kind)).toEqual([
			"snapshot",
			"locked",
		]);
		expect(row.unknown[0].serviceId).toBe("scanner-ingest");
		expect(row.unknown[1].serviceId).toBeUndefined();
	});

	test("keeps the device's own refusal for the popover", () => {
		const refused = rowOf("app_crm_sync", "evt_crm_watch").elsewhere.find(
			(entry) => entry.state === "cant_here",
		);
		expect(refused?.deviceId).toBe("edge-berlin-01");
		expect(refused?.reason).toContain("sandboxed");
	});

	test("events that can't run on devices have no row", () => {
		const rows = runsOnRows(viewOf("app_crm_sync"));
		expect([...rows.keys()].sort()).toEqual([
			"evt_crm_nightly",
			"evt_crm_watch",
			"evt_crm_webhook",
		]);
	});

	test("the cell's counts don't depend on listing every device as a column", () => {
		const devices = sampleDevices();
		const narrow = runsOnRows(
			buildAppView({
				app: APPS.app_invoice_ai,
				devices,
				placements: PLACEMENTS.app_invoice_ai,
			}),
		);
		for (const [eventId, row] of runsOnRows(viewOf("app_invoice_ai"))) {
			expect(devicesOf(narrow.get(eventId)?.served ?? [])).toEqual(
				devicesOf(row.served),
			);
			expect(devicesOf(narrow.get(eventId)?.unknown ?? [])).toEqual(
				devicesOf(row.unknown),
			);
		}
	});
});

describe("runsOnDeviceNames", () => {
	test("names every device the app view mentions", () => {
		const names = runsOnDeviceNames(viewOf("app_invoice_ai"));
		expect([...names.keys()].sort()).toEqual([
			"cold-storage-nas",
			"edge-berlin-01",
			"lab-gpu-02",
			"studio-mac-mini",
			"warehouse-pi",
		]);
	});
});
