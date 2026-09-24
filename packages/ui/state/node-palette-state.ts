import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

const RECENT_CAPACITY = 12;

interface NodePaletteState {
	/** Palette entry keys, most recently placed first. */
	recents: string[];
	recordPlacement: (key: string) => void;
}

export const useNodePaletteStore = create<NodePaletteState>()(
	persist(
		(set) => ({
			recents: [],
			recordPlacement: (key) =>
				set((state) => ({
					recents: [key, ...state.recents.filter((k) => k !== key)].slice(
						0,
						RECENT_CAPACITY,
					),
				})),
		}),
		{
			name: "node-palette-storage",
			storage: createJSONStorage(() => localStorage),
			partialize: (state) => ({ recents: state.recents }),
		},
	),
);
