import { expect, test } from "bun:test";
import { createDeployRun } from "../../../lib/device-management/model/deploy-run";
import type { DeployDevice } from "../devices/deploy/deploy-facts";
import {
	deploymentIsBusy,
	deviceReadyForEvent,
	matchesDeviceFilter,
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

test("a selection becoming blocked or disappearing is rejected", () => {
	const selected = new Set(["edge"]);
	expect(unavailableSelectedDevice(selected, [device()])).toBeUndefined();
	expect(
		unavailableSelectedDevice(selected, [
			device({ gate: { code: "offline" } as DeployDevice["gate"] }),
		]),
	).toBe("edge");
	expect(unavailableSelectedDevice(selected, [])).toBe("edge");
	expect(unavailableSelectedDevice(new Set(), [])).toBeUndefined();
});
