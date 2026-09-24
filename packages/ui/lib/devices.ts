import type { IApiState } from "../state/backend-state/api-state";
import type { IProfile } from "../types";

export interface DeviceStatus {
	device_id: string;
	owner_id: string;
	name: string;
	status: "active" | "revoked";
	registered_at: number;
	last_seen_at: number | null;
	auth_epoch: number;
}

export function listDevices(api: IApiState, profile: IProfile) {
	return api.get<DeviceStatus[]>(profile, "devices");
}

export function revokeDevice(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
): Promise<void> {
	return api.del<void>(profile, `devices/${encodeURIComponent(deviceId)}`);
}

/** A recent heartbeat is evidence of contact, not a live management connection. */
export function hasRecentDeviceContact(
	device: DeviceStatus,
	nowMilliseconds: number,
): boolean {
	if (device.status !== "active" || device.last_seen_at === null) return false;
	const elapsed = nowMilliseconds - device.last_seen_at * 1_000;
	return elapsed >= 0 && elapsed <= 120_000;
}
