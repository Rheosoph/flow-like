import { del, get, set } from "idb-keyval";
import { create } from "zustand";
import {
	type StateStorage,
	createJSONStorage,
	persist,
} from "zustand/middleware";

interface IFlowBoardParentState {
	boardParents: {
		[boardId: string]: string;
	};
	addBoardParent: (boardId: string, parentId: string) => void;
	/**
	 * Register many boards under one parent in a single write.
	 *
	 * `persist` writes the whole state after *every* `set`, so calling
	 * `addBoardParent` in a loop opens one IndexedDB transaction per board. On
	 * desktop those land on the SQLite-backed shim, where a burst of concurrent
	 * write transactions contends for the database lock. No-ops when every entry
	 * already points at the same parent, so a re-render cannot cause a write.
	 */
	addBoardParents: (parents: Readonly<Record<string, string>>) => void;
	removeBoardParent: (boardId: string) => void;
}

const storage: StateStorage = {
	getItem: async (name: string): Promise<string | null> => {
		return (await get(name)) ?? null;
	},
	setItem: async (name: string, value: string): Promise<void> => {
		await set(name, value);
	},
	removeItem: async (name: string): Promise<void> => {
		await del(name);
	},
};

export const useFlowBoardParentState = create(
	persist<IFlowBoardParentState>(
		(set, get) => ({
			boardParents: {},
			addBoardParent: (boardId, parentLink) => {
				set((state) => {
					return {
						boardParents: {
							...state.boardParents,
							[boardId]: parentLink,
						},
					};
				});
			},
			addBoardParents: (parents) => {
				const current = get().boardParents;
				const changed = Object.entries(parents).filter(
					([boardId, parentLink]) => current[boardId] !== parentLink,
				);
				if (changed.length === 0) return;
				set({
					boardParents: {
						...current,
						...Object.fromEntries(changed),
					},
				});
			},
			removeBoardParent: (boardId) => {
				set((state) => {
					const newBoardParents = { ...state.boardParents };
					delete newBoardParents[boardId];
					return {
						boardParents: newBoardParents,
					};
				});
			},
		}),
		{
			name: "flow-board-parent",
			storage: createJSONStorage(() => storage),
		},
	),
);
