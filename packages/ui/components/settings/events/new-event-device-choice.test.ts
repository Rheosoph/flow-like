import { expect, test } from "bun:test";
import { createDeployRun } from "../../../lib/device-management/model/deploy-run";
import type { DeployDevice } from "../devices/deploy/deploy-facts";
import {
	type CreateBlockInput,
	createBlock,
	defaultDeviceFilter,
	deploymentIsBusy,
	deviceFilterCounts,
	deviceReadyForEvent,
	indexDevices,
	listDevices,
	matchesDeviceFilter,
	pickableDevices,
	resultFooterAction,
	stepTab,
	unavailableSelectedDevice,
} from "./new-event-device-choice";

test("dismissal waits for active work but remains available during setup and failures", () => {
	const idle = createDeployRun({
		id: "deployment",
		order: "all",
		stopOnFail: true,
		shared: "prepare",
		rows: [{ target: "edge/event", deviceId: "edge", phases: ["start"] }],
	});
	expect(deploymentIsBusy(null)).toBe(false);
	expect(deploymentIsBusy(idle)).toBe(false);
	expect(deploymentIsBusy({ ...idle, status: "running" })).toBe(true);
	expect(deploymentIsBusy({ ...idle, status: "held" })).toBe(false);
	expect(deploymentIsBusy({ ...idle, status: "finished" })).toBe(false);
	expect(
		deploymentIsBusy({
			...idle,
			status: "held",
			shared: { phase: "prepare", state: "active" },
		}),
	).toBe(true);
	expect(
		deploymentIsBusy({
			...idle,
			status: "held",
			rows: idle.rows.map((row) => ({ ...row, state: "active" })),
		}),
	).toBe(true);
});

function device(patch: Partial<DeployDevice> = {}): DeployDevice {
	return {
		id: "edge",
		name: "Edge",
		presence: { kind: "online" },
		locked: false,
		gate: null,
		services: null,
		...patch,
	} as DeployDevice;
}

test("Ready excludes locked, offline and gated devices", () => {
	expect(deviceReadyForEvent(device())).toBe(true);
	expect(deviceReadyForEvent(device({ locked: true }))).toBe(false);
	expect(deviceReadyForEvent(device({ presence: { kind: "offline" } }))).toBe(
		false,
	);
	expect(
		deviceReadyForEvent(
			device({ gate: { code: "offline" } as DeployDevice["gate"] }),
		),
	).toBe(false);
});

test("Runs this app requires a known running service for this app", () => {
	const service = (projectId: string, desired: string) =>
		({ projectId, desired }) as NonNullable<DeployDevice["services"]>[number];
	expect(matchesDeviceFilter(device(), "app", "app")).toBe(false);
	expect(
		matchesDeviceFilter(
			device({ services: [service("other", "running")] }),
			"app",
			"app",
		),
	).toBe(false);
	expect(
		matchesDeviceFilter(
			device({ services: [service("app", "stopped")] }),
			"app",
			"app",
		),
	).toBe(false);
	expect(
		matchesDeviceFilter(
			device({ services: [service("app", "running")] }),
			"app",
			"app",
		),
	).toBe(true);
});

const GATE = { code: "offline" } as DeployDevice["gate"];

test("a selection becoming blocked or disappearing is rejected", () => {
	const selected = new Set(["edge"]);
	expect(
		unavailableSelectedDevice(selected, indexDevices([device()])),
	).toBeUndefined();
	expect(
		unavailableSelectedDevice(selected, indexDevices([device({ gate: GATE })])),
	).toBe("edge");
	expect(unavailableSelectedDevice(selected, indexDevices([]))).toBe("edge");
	expect(
		unavailableSelectedDevice(new Set(), indexDevices([])),
	).toBeUndefined();
});

test("filter counts come from one pass and Ready is the default only when something is ready", () => {
	const service = {
		projectId: "app",
		desired: "running",
	} as NonNullable<DeployDevice["services"]>[number];
	const fleet = [
		device({ id: "a" }),
		device({ id: "b", services: [service] }),
		device({ id: "c", presence: { kind: "offline" } }),
		device({ id: "d", gate: GATE }),
	];
	const counts = deviceFilterCounts(fleet, "app");
	expect(counts).toEqual({ ready: 2, app: 1, all: 4 });
	expect(defaultDeviceFilter(counts)).toBe("ready");
	expect(
		defaultDeviceFilter(deviceFilterCounts([fleet[2] as DeployDevice], "app")),
	).toBe("all");
	expect(defaultDeviceFilter(deviceFilterCounts([], "app"))).toBe("all");
});

test("the list narrows by filter, search and the selected-only view", () => {
	const fleet = [
		device({ id: "a", name: "Edge Berlin", platform: "linux" }),
		device({ id: "b", name: "Studio", platform: "macos" }),
		device({ id: "c", name: "Offline box", presence: { kind: "offline" } }),
	];
	const base = {
		filter: "all",
		appId: "app",
		search: "",
		selectedOnly: false,
		selected: new Set<string>(),
	} as const;
	const ids = (query: Partial<Parameters<typeof listDevices>[1]>) =>
		listDevices(fleet, { ...base, ...query }).map((row) => row.id);
	expect(ids({})).toEqual(["a", "b", "c"]);
	expect(ids({ filter: "ready" })).toEqual(["a", "b"]);
	expect(ids({ search: "mac" })).toEqual(["b"]);
	expect(ids({ selectedOnly: true, selected: new Set(["c"]) })).toEqual(["c"]);
});

test("Pick all takes the ready devices of the list that are not selected yet", () => {
	const fleet = [
		device({ id: "a" }),
		device({ id: "b" }),
		device({ id: "c", gate: GATE }),
		device({ id: "d", locked: true }),
	];
	expect(pickableDevices(fleet, new Set())).toEqual(["a", "b"]);
	expect(pickableDevices(fleet, new Set(["a"]))).toEqual(["b"]);
	expect(pickableDevices(fleet, new Set(["a", "b"]))).toEqual([]);
});

const READY: CreateBlockInput = {
	hubBlocked: false,
	hubLoading: false,
	hubFailed: false,
	devicesLoading: false,
	devicesFailed: false,
	unsupported: false,
	selected: 1,
	singleDevice: false,
	unavailable: undefined,
};

test("the footer names the first reason Create & deploy cannot run", () => {
	const block = (patch: Partial<CreateBlockInput>) =>
		createBlock({ ...READY, ...patch });
	expect(block({})).toBeNull();
	expect(block({ hostDisabled: true })).toBe("form_incomplete");
	expect(block({ hostDisabled: true, selected: 0 })).toBe("form_incomplete");
	expect(block({ selected: 0 })).toBe("no_device");
	expect(block({ selected: 0, unsupported: true })).toBe("unsupported");
	expect(block({ devicesLoading: true, selected: 0 })).toBe("devices_loading");
	expect(block({ devicesFailed: true })).toBe("devices_failed");
	expect(block({ hubBlocked: true, hubLoading: true })).toBe("hub_loading");
	expect(block({ hubBlocked: true, hubFailed: true })).toBe("hub_failed");
	expect(block({ hubBlocked: true })).toBe("hub_outdated");
	expect(block({ singleDevice: true, selected: 2 })).toBe("too_many");
	expect(block({ unavailable: "edge" })).toBe("device_blocked");
	expect(block({ singleDevice: true, selected: 1 })).toBeNull();
});

test("a saved event opens on the first step that needs input, else Review", () => {
	expect(stepTab(undefined)).toBe("review");
	expect(stepTab("access_cost")).toBe("access_cost");
	expect(stepTab("settings")).toBe("settings");
	expect(stepTab("what")).toBe("where");
	expect(stepTab("how")).toBe("where");
});

test("the footer says Done only when every device took the event", () => {
	expect(resultFooterAction(null)).toBeNull();
	expect(resultFooterAction({ outcome: "all" })).toBe("done");
	expect(resultFooterAction({ outcome: "partial" })).toBeNull();
	expect(resultFooterAction({ outcome: "none" })).toBeNull();
});
