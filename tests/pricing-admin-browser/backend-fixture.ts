import { create } from "zustand";
const emptyBackend = { userState: {}, apiState: {} };
export const useBackendStore = create<{
	backend: any;
	setBackend: (backend: any) => void;
}>((set) => ({
	backend: emptyBackend,
	setBackend: (backend) => set({ backend }),
}));
export const useBackend = () => useBackendStore((state) => state.backend);
export type IBackendState = any;
