import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { generateFleet } from "../../../../lib/device-management/model/__fixtures__/sample-fleet-200";
import {
	allByRole,
	byRole,
	click,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type {
	MountDevicesOptions,
	MountedDevices,
} from "../testing/mount-devices";
import type { PlaneSegmentId } from "../workspace/use-attention";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { useOverlayStore } = await import("../workspace/overlay-store");

const { warehouse, edge } = SAMPLE_IDS;

/** R3: no check code, no gate code, no wire value. */
const MACHINE_WORDS = /\b[GD]\d{1,2}\b|\b[a-z]+_[a-z_]+\b/;
const PLANES = ["hub", "status", "live", "local", "device", "certificates"];
const NAMES = [
	"Hub",
	"Encrypted status",
	"Live",
	"This computer",
	"On-device only",
	"Certificates",
];

afterEach(async () => {
	await act(async () => {
		useOverlayStore.getState().close();
	});
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

async function showPlane(mounted: MountedDevices, plane: PlaneSegmentId) {
	await act(async () => {
		useOverlayStore.getState().openPlane(plane);
	});
	await mounted.settle();
}

async function openPlane(
	plane: PlaneSegmentId,
	options: MountDevicesOptions = {},
) {
	const mounted = await mountDevices(<div />, { overlays: true, ...options });
	await showPlane(mounted, plane);
	return mounted;
}

function sheet() {
	return byRole("dialog");
}

function item(plane: PlaneSegmentId) {
	return sheet().querySelector(`li[data-plane="${plane}"]`) as HTMLElement;
}

function perDevice(plane: PlaneSegmentId) {
	return byRole("button", "Per device", item(plane));
}

function textOf(node: Element) {
	return node.textContent ?? "";
}

function planeOf(node: Element) {
	return node.getAttribute("data-plane");
}

function tableRows(plane: PlaneSegmentId) {
	return Array.from(item(plane).querySelectorAll("tbody tr"), textOf);
}

function tableRow(plane: PlaneSegmentId, device: string) {
	const rows = item(plane).querySelectorAll<HTMLElement>("tbody tr");
	for (const row of rows) if (textOf(row).includes(device)) return row;
	throw new Error(`${device} has no row under ${plane}`);
}

function lastHref(mounted: MountedDevices) {
	return mounted.navigations.at(-1)?.href;
}

describe("data sources sheet", () => {
	test("lists all six sources with what they are, how fresh and who can read them", async () => {
		await openPlane("hub");
		expect(textOf(sheet())).toContain("Data sources");
		expect(textOf(sheet())).toContain(
			"Where each part of this page comes from and how fresh it is.",
		);
		const list = byRole("list", "Data sources");
		const planes = Array.from(list.querySelectorAll(":scope > li"), planeOf);
		expect(planes).toEqual(PLANES);
		const headings = allByRole("heading", undefined, list).map(textOf);
		expect(headings).toHaveLength(6);
		for (const [index, name] of NAMES.entries())
			expect(headings[index]).toContain(name);
		expect(textOf(item("hub"))).toContain("Refreshed every 30 s.");
		expect(textOf(item("hub"))).toContain(
			"Who can read it: Any signed-in account with access, without a device password.",
		);
		expect(textOf(item("status"))).toContain(
			"Who can read it: Only this computer, with the device's keys unlocked.",
		);
		expect(sheet().querySelectorAll("[data-dv-primary]")).toHaveLength(0);
		expect(textOf(sheet())).not.toMatch(MACHINE_WORDS);
		expect(queryByRole("alert", undefined, sheet())).toBeNull();
	});

	test("only the source it was opened for shows its devices; the others open on demand", async () => {
		await openPlane("status");
		expect(perDevice("status").getAttribute("aria-expanded")).toBe("true");
		expect(perDevice("hub").getAttribute("aria-expanded")).toBe("false");
		expect(sheet().querySelectorAll("table")).toHaveLength(1);
		expect(textOf(tableRow("status", "lab-gpu-02"))).toContain("Locked");

		await click(perDevice("hub"));
		expect(perDevice("hub").getAttribute("aria-expanded")).toBe("true");
		expect(sheet().querySelectorAll("table")).toHaveLength(2);
		expect(textOf(tableRow("hub", "warehouse-pi"))).toContain("Offline");
		await click(perDevice("status"));
		expect(sheet().querySelectorAll("table")).toHaveLength(1);
	});

	test("a device name opens the device, a source's action opens its place", async () => {
		const mounted = await openPlane("hub");
		await click(byRole("link", "edge-berlin-01", item("hub")));
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(lastHref(mounted)).toBe(
			`/settings/devices?device=${edge}&tab=overview`,
		);

		await showPlane(mounted, "certificates");
		await click(byRole("button", "Hub status", item("hub")));
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(lastHref(mounted)).toBe("/settings/devices?view=hub");
	});

	test("Unlock several… and Diagnose replace the sheet", async () => {
		const mounted = await openPlane("device");
		const offline = tableRow("device", "warehouse-pi");
		await click(byRole("button", "Diagnose", offline));
		await mounted.settle();
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "diagnose",
			deviceId: warehouse,
		});
		expect(textOf(sheet())).toContain("Diagnose warehouse-pi");

		await showPlane(mounted, "status");
		await click(byRole("button", "Unlock several…", item("status")));
		await mounted.settle();
		expect(useOverlayStore.getState().overlay.kind).toBe("unlock_several");
		expect(textOf(sheet())).toContain("Unlock several devices");
	});

	test("an older hub: every source still renders, nothing errors", async () => {
		await openPlane("certificates", { hubVersion: "old" });
		expect(sheet().querySelectorAll("li[data-plane]")).toHaveLength(6);
		expect(queryByRole("alert", undefined, sheet())).toBeNull();
		expect(item("certificates").querySelector("table")).not.toBeNull();
	});

	test("a 200-device fleet shows 25 rows per source until Show all", async () => {
		const seed = generateFleet(200).input;
		await openPlane("hub", { seed, unlock: "none" });
		expect(tableRows("hub")).toHaveLength(25);
		await click(byRole("button", "Show all 200", item("hub")));
		expect(tableRows("hub")).toHaveLength(200);
	});
});
