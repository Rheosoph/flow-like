import type { InventoryScope } from "../types";
import type { AttentionInputExt } from "./attention";
import { type DeviceFacts, fleetFacts, keysLocked } from "./device-view";
import type { ServiceView } from "./types";

/** Whose status the page can vouch for (N1/N4 "Status from 3 of 5 devices · 2 locked"). Ids are device ids. */
export interface Coverage {
	/** Non-revoked devices the viewer can see. */
	total: number;
	readable: number;
	live: number;
	snapshot: number;
	/** Not readable, for any reason. */
	unknown: string[];
	/** Not readable because keys here are locked. */
	locked: string[];
	/** Not readable because this computer has no keys. */
	noKeys: string[];
	/** App scope: shared devices whose access doesn't cover the app. */
	noAccess: string[];
	/** Shared devices whose access covers only part of the device. */
	partial: string[];
	/** App scope, readable devices only: runs the app or not. */
	deployed: string[];
	notDeployed: string[];
}

/** BG22 my-access when the hub has it, else the verified policy after unlock. */
function myScopes(
	input: AttentionInputExt,
	device: DeviceFacts,
): InventoryScope[] | undefined {
	if (device.relationship !== "shared") return undefined;
	const grants = input.myAccess?.[device.id]?.grants;
	if (grants?.length) return grants.map((grant) => grant.scope);
	const mine = input.fleet[device.id]?.policy?.myGrant;
	return mine ? [mine.scope] : undefined;
}

function covers(scope: InventoryScope, appId: string) {
	return scope.kind === "device" || scope.project_id === appId;
}

const outsideApp = (scopes: InventoryScope[] | undefined, appId?: string) =>
	!!appId && !!scopes && !scopes.some((scope) => covers(scope, appId));

const partOfDevice = (scopes: InventoryScope[] | undefined) =>
	!!scopes && !scopes.some((scope) => scope.kind === "device");

const unreadableReason = (device: DeviceFacts) => {
	if (keysLocked(device.keys)) return "locked";
	return device.keys.state === "none" ? "noKeys" : undefined;
};

const countReadable = (
	result: Coverage,
	device: DeviceFacts,
	services: readonly ServiceView[],
	appId?: string,
) => {
	result.readable++;
	if (device.liveOpen) result.live++;
	else result.snapshot++;
	if (!appId) return;
	const runs = services.some((service) => service.projectId === appId);
	(runs ? result.deployed : result.notDeployed).push(device.id);
};

const countDevice = (
	input: AttentionInputExt,
	result: Coverage,
	device: DeviceFacts,
	appId?: string,
) => {
	result.total++;
	const scopes = myScopes(input, device);
	if (outsideApp(scopes, appId)) {
		result.noAccess.push(device.id);
		return;
	}
	if (partOfDevice(scopes)) result.partial.push(device.id);
	if (Array.isArray(device.services)) {
		countReadable(result, device, device.services, appId);
		return;
	}
	result.unknown.push(device.id);
	const reason = unreadableReason(device);
	if (reason) result[reason].push(device.id);
};

export function coverage(input: AttentionInputExt, appId?: string): Coverage {
	const result: Coverage = {
		total: 0,
		readable: 0,
		live: 0,
		snapshot: 0,
		unknown: [],
		locked: [],
		noKeys: [],
		noAccess: [],
		partial: [],
		deployed: [],
		notDeployed: [],
	};
	for (const device of fleetFacts(input).devices)
		if (device.active) countDevice(input, result, device, appId);
	return result;
}
