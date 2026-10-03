"use client";

import { create } from "zustand";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import type { PlaneSegmentId } from "./use-attention";

export interface UnlockRequest {
	/** Also open a live session once the keys are open. */
	connectLive?: boolean;
	/** Where to land after unlocking (deep link to a locked section). */
	returnTo?: DevicesRoute;
}

/** "Run now…" of one quick action or form a service runs (design R2 §6.5). */
export interface RunNowRequest {
	deviceId: string;
	serviceId: string;
	eventId: string;
	/** Shows this run you started instead of a new form ("Runs you started", the tray). */
	operationId?: string;
}

export type OverlayState =
	| { kind: "none" }
	| ({ kind: "unlock"; deviceId: string } & UnlockRequest)
	| { kind: "unlock_several" }
	| { kind: "diagnose"; deviceId: string; serviceId?: string }
	| { kind: "plane"; plane: PlaneSegmentId }
	| ({ kind: "run_now" } & RunNowRequest);

interface OverlayStore {
	overlay: OverlayState;
	openUnlock(deviceId: string, request?: UnlockRequest): void;
	openUnlockSeveral(): void;
	openDiagnose(deviceId: string, serviceId?: string): void;
	openPlane(plane: PlaneSegmentId): void;
	openRunNow(request: RunNowRequest): void;
	close(): void;
}

const NONE: OverlayState = { kind: "none" };

/**
 * Which area overlay is open (IA §6.4). Overlays have no URL; the host that
 * renders them (`AreaOverlays`) reads this store, and callers work without one
 * mounted: the request waits until a host appears.
 */
export const useOverlayStore = create<OverlayStore>((set) => ({
	overlay: NONE,
	openUnlock: (deviceId, request = {}) =>
		set({ overlay: { kind: "unlock", deviceId, ...request } }),
	openUnlockSeveral: () => set({ overlay: { kind: "unlock_several" } }),
	openDiagnose: (deviceId, serviceId) =>
		set({
			overlay: {
				kind: "diagnose",
				deviceId,
				...(serviceId ? { serviceId } : {}),
			},
		}),
	openPlane: (plane) => set({ overlay: { kind: "plane", plane } }),
	openRunNow: ({ deviceId, serviceId, eventId, operationId }) =>
		set({
			overlay: {
				kind: "run_now",
				deviceId,
				serviceId,
				eventId,
				...(operationId ? { operationId } : {}),
			},
		}),
	close: () => set({ overlay: NONE }),
}));

export function useOverlay(): OverlayStore {
	return useOverlayStore();
}
