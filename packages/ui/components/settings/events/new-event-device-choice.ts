import type { DeployResult } from "../../../lib/device-management/model/deploy-plan";
import type { DeployRunState } from "../../../lib/device-management/model/deploy-run";
import type { DeployDevice } from "../devices/deploy/deploy-facts";
import type { DeployStepId } from "../devices/deploy/step-props";

/**
 * What the dialog footer offers once a rollout has a result. "Done" means the event is deployed,
 * so it appears only when every device took it; after a partial or failed rollout the rollout
 * block's Retry stays the primary action and Close keeps the event for later.
 */
export function resultFooterAction(
	result: Pick<DeployResult, "outcome"> | null,
): "done" | null {
	return result?.outcome === "all" ? "done" : null;
}

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

export type DeviceIndex = ReadonlyMap<string, DeployDevice>;

export function indexDevices(devices: readonly DeployDevice[]): DeviceIndex {
	return new Map(devices.map((device) => [device.id, device]));
}

/** Keep a missing or newly blocked selection visible as a problem until the user removes it. */
export function unavailableSelectedDevice(
	selected: ReadonlySet<string>,
	devices: DeviceIndex,
): string | undefined {
	for (const id of selected) {
		const device = devices.get(id);
		if (!device || device.gate) return id;
	}
	return undefined;
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

export type DeviceFilterCounts = Record<NewEventDeviceFilter, number>;

export function deviceFilterCounts(
	devices: readonly DeployDevice[],
	appId: string,
): DeviceFilterCounts {
	const counts: DeviceFilterCounts = { ready: 0, app: 0, all: devices.length };
	for (const device of devices) {
		if (deviceReadyForEvent(device)) counts.ready += 1;
		if (deviceRunsApp(device, appId)) counts.app += 1;
	}
	return counts;
}

/** Ready is the working set whenever anything is ready; an all-offline fleet would otherwise open on an empty list. */
export function defaultDeviceFilter(
	counts: DeviceFilterCounts,
): NewEventDeviceFilter {
	return counts.ready > 0 ? "ready" : "all";
}

export interface DeviceListQuery {
	filter: NewEventDeviceFilter;
	appId: string;
	/** Lower-cased and trimmed. */
	search: string;
	selectedOnly: boolean;
	selected: ReadonlySet<string>;
}

export function listDevices(
	devices: readonly DeployDevice[],
	{ filter, appId, search, selectedOnly, selected }: DeviceListQuery,
): DeployDevice[] {
	return devices.filter(
		(device) =>
			matchesDeviceFilter(device, filter, appId) &&
			(!selectedOnly || selected.has(device.id)) &&
			(!search ||
				`${device.name} ${device.platform ?? ""}`
					.toLowerCase()
					.includes(search)),
	);
}

/** What "Pick all shown" adds: ready devices of the list that are not selected yet. */
export function pickableDevices(
	shown: readonly DeployDevice[],
	selected: ReadonlySet<string>,
): string[] {
	return shown
		.filter((device) => deviceReadyForEvent(device) && !selected.has(device.id))
		.map((device) => device.id);
}

/** The tab of a deploy step (What and How live in Devices); no step means nothing blocks, so Review. */
export function stepTab(step: DeployStepId | undefined): DeployStepId {
	if (!step) return "review";
	return step === "what" || step === "how" ? "where" : step;
}

export const DEVICE_PAGE = 100;
export const CHIP_LIMIT = 3;

export type CreateBlockCode =
	| "form_incomplete"
	| "unsupported"
	| "hub_loading"
	| "hub_failed"
	| "hub_outdated"
	| "devices_failed"
	| "devices_loading"
	| "no_device"
	| "device_blocked"
	| "too_many";

export interface CreateBlockInput {
	/** The host form cannot be submitted (incomplete or busy elsewhere). */
	hostDisabled?: boolean;
	/** An online app whose hub cannot create device events (yet). */
	hubBlocked: boolean;
	hubLoading: boolean;
	hubFailed: boolean;
	devicesLoading: boolean;
	devicesFailed: boolean;
	unsupported: boolean;
	selected: number;
	singleDevice: boolean;
	/** The id of the first selected device that is missing or gated. */
	unavailable: string | undefined;
}

/** The first reason "Create & deploy" cannot run, or null; the footer words it. */
export function createBlock(input: CreateBlockInput): CreateBlockCode | null {
	const hub: CreateBlockCode = input.hubLoading
		? "hub_loading"
		: input.hubFailed
			? "hub_failed"
			: "hub_outdated";
	const checks: [boolean, CreateBlockCode][] = [
		[!!input.hostDisabled, "form_incomplete"],
		[input.unsupported, "unsupported"],
		[input.devicesLoading, "devices_loading"],
		[input.devicesFailed, "devices_failed"],
		[input.hubBlocked, hub],
		[input.singleDevice && input.selected > 1, "too_many"],
		[!!input.unavailable, "device_blocked"],
		[input.selected === 0, "no_device"],
	];
	return checks.find(([failed]) => failed)?.[1] ?? null;
}
