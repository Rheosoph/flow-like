"use client";

import { createId } from "@paralleldrive/cuid2";
import { useCallback } from "react";
import {
	type IGenericCommand,
	removeLayerCommand,
	upsertLayerCommand,
} from "../../lib";
import { type ILayer, ILayerType } from "../../lib/schema/flow/board";

/**
 * Creating, renaming, moving and deleting module layers — every write the file
 * tree and the tab strip perform, so the two cannot drift on validation or on
 * what a delete does to the nodes inside.
 */
export function useModuleCommands(
	executeCommand: (
		command: IGenericCommand,
		append: boolean,
	) => Promise<unknown>,
) {
	const createModule = useCallback(
		async (name: string, parentId: string | null) => {
			const layer: ILayer = {
				id: createId(),
				name,
				type: ILayerType.Module,
				coordinates: [0, 0, 0],
				nodes: {},
				pins: {},
				variables: {},
				comments: {},
				// The backend takes the parent of a *new* layer from `current_layer`;
				// `parent_id` is what every local reader goes by. Both or the module lands
				// somewhere else than the tab it was created from.
				parent_id: parentId,
				color: null,
				comment: null,
				error: null,
				category: null,
			};
			await executeCommand(
				upsertLayerCommand({
					layer,
					node_ids: [],
					current_layer: parentId,
				}),
				false,
			);
			return layer;
		},
		[executeCommand],
	);

	const renameModule = useCallback(
		async (layer: ILayer, name: string) => {
			await executeCommand(
				upsertLayerCommand({ layer: { ...layer, name }, node_ids: [] }),
				false,
			);
		},
		[executeCommand],
	);

	const deleteModule = useCallback(
		async (layer: ILayer, preserveNodes: boolean) => {
			await executeCommand(
				removeLayerCommand({
					layer,
					preserve_nodes: preserveNodes,
					child_layers: [],
					layer_nodes: [],
					layers: [],
					nodes: [],
				}),
				false,
			);
		},
		[executeCommand],
	);

	const moveModule = useCallback(
		async (layer: ILayer, parentId: string | null) => {
			if ((layer.parent_id ?? null) === parentId) return;
			await executeCommand(
				upsertLayerCommand({
					layer: { ...layer, parent_id: parentId },
					node_ids: [],
					current_layer: parentId,
				}),
				false,
			);
		},
		[executeCommand],
	);

	return { createModule, renameModule, moveModule, deleteModule };
}
