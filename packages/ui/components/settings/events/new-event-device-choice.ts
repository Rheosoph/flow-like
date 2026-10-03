import type { DeployRunState } from "../../../lib/device-management/model/deploy-run";
import type { DeployDevice } from "../devices/deploy/deploy-facts";

export function deploymentIsBusy(run: DeployRunState | null): boolean {
	return (
		!!run &&
		(run.status === "running" ||
			run.shared?.state === "active" ||
			run.rows.some((row) => row.state === "active"))
	);
}

export type NewEventDeviceFilter = "ready" | "app" | "all";

export function deviceReadyForEvent(device: DeployDevice): boolean {
	return !device.gate && !device.locked && device.presence.kind === "online";
}

export function deviceRunsApp(device: DeployDevice, appId: string): boolean {
	return (
		device.services?.some(
			(service) => service.projectId === appId && service.desired === "running",
		) ?? false
	);
}

/** Keep a missing or newly blocked selection visible as a problem until the user removes it. */
export function unavailableSelectedDevice(
	selected: ReadonlySet<string>,
	devices: readonly DeployDevice[],
): string | undefined {
	return [...selected].find((id) => {
		const device = devices.find((row) => row.id === id);
		return !device || !!device.gate;
	});
}

export function matchesDeviceFilter(
	device: DeployDevice,
	filter: NewEventDeviceFilter,
	appId: string,
): boolean {
	if (filter === "ready") return deviceReadyForEvent(device);
	if (filter === "app") return deviceRunsApp(device, appId);
	return true;
}
