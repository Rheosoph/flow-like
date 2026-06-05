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
