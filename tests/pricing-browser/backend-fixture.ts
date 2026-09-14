import { create } from "zustand";
import type { IBackendState } from "../../packages/ui/state/backend-state";

// Only the API boundary is replaced. Screens render the production components.
export const useBackendStore = create<{
	backend: IBackendState;
	setBackend: (backend: IBackendState) => void;
}>((set) => ({
	backend: {
		userState: {},
		apiState: {},
		appState: {},
		bitState: {},
	} as IBackendState,
	setBackend: (backend) => set({ backend }),
}));
export const useBackend = () => useBackendStore((state) => state.backend);
export const useBackendReady = () => true;
