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

export type OverlayState =
	| { kind: "none" }
	| ({ kind: "unlock"; deviceId: string } & UnlockRequest)
	| { kind: "unlock_several" }
	| { kind: "diagnose"; deviceId: string; serviceId?: string }
	| { kind: "plane"; plane: PlaneSegmentId };

interface OverlayStore {
	overlay: OverlayState;
	openUnlock(deviceId: string, request?: UnlockRequest): void;
	openUnlockSeveral(): void;
	openDiagnose(deviceId: string, serviceId?: string): void;
	openPlane(plane: PlaneSegmentId): void;
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
	close: () => set({ overlay: NONE }),
}));

export function useOverlay(): OverlayStore {
	return useOverlayStore();
}
