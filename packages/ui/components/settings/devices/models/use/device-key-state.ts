"use client";

import { useSyncExternalStore } from "react";
import {
	type NativeKeyMirror,
	nativeKeys,
} from "../../../../../lib/device-management/native-client";

/**
 * Whether runs in this app reach a device's models without asking: the
 * desktop app holds its keys (`unlocked`), or a run asks for its password
 * first (`locked`).
 */
export type DeviceKeyState = "unlocked" | "locked";

export interface HeldDevices {
	subscribe(listener: () => void): () => void;
	/** `undefined` off the desktop app and until its first answer. */
	snapshot(): ReadonlySet<string> | undefined;
}

/** One watch of the desktop app's held keys, shared by every picker on screen. */
export function createHeldDevices(
	keys: Pick<NativeKeyMirror, "watch">,
): HeldDevices {
	const listeners = new Set<() => void>();
	let held: ReadonlySet<string> | undefined;
	let stop: (() => void) | undefined;
	const publish = (next: ReadonlySet<string>) => {
		held = next;
		for (const listener of listeners) listener();
	};
	return {
		subscribe(listener) {
			listeners.add(listener);
			stop ??= keys.watch({
				held: (devices) =>
					publish(new Set(devices.map((device) => device.deviceId))),
				lockedAll: () => publish(new Set()),
			});
			return () => {
				listeners.delete(listener);
				if (listeners.size > 0) return;
				stop?.();
				stop = undefined;
				held = undefined;
			};
		},
		snapshot: () => held,
	};
}

const HELD = createHeldDevices(nativeKeys);
const unknown = () => undefined;

/** `undefined` where nothing says it: off the desktop app, or no device. */
export function useDeviceKeyState(
	deviceId: string | undefined,
	held: HeldDevices = HELD,
): DeviceKeyState | undefined {
	const devices = useSyncExternalStore(held.subscribe, held.snapshot, unknown);
	if (!deviceId || !devices) return undefined;
	return devices.has(deviceId) ? "unlocked" : "locked";
}
