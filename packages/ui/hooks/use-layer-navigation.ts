import type { UseQueryResult } from "@tanstack/react-query";
import type { ReactFlowInstance } from "@xyflow/react";
import { useCallback } from "react";
import type { IBoard, ILayer } from "../lib/schema/flow/board";

interface UseLayerNavigationProps {
	board: UseQueryResult<IBoard>;
	layerPath: string | undefined;
	setCurrentLayer: (layer: string | undefined) => void;
	setLayerPath: (path: string | undefined | ((old?: string) => string)) => void;
	saveViewport: () => Promise<void>;
	fitView: ReactFlowInstance["fitView"];
}

export function useLayerNavigation({
	board,
	layerPath,
	setCurrentLayer,
	setLayerPath,
	saveViewport,
	fitView,
}: UseLayerNavigationProps) {
	const focusNode = useCallback(
		(nodeId: string) => {
			const node = board.data?.nodes[nodeId];
			if (!node) {
				console.error("Node not found:", nodeId);
				return;
			}

			const layers = board.data?.layers ?? {};
			const layerTree: string[] = [];
			let parentLayer = node.layer;
			let iteration = 0;

			while (parentLayer && iteration < 40) {
				iteration++;
				const layer = layers[parentLayer];
				if (!layer) break;
				layerTree.push(layer.id);
				parentLayer = layer.parent_id;
			}

			if (layerTree.length > 0) {
				setCurrentLayer(layerTree[layerTree.length - 1]);
				setLayerPath(layerTree.slice().reverse().join("/"));
			} else {
				setCurrentLayer(undefined);
				setLayerPath(undefined);
			}

			requestAnimationFrame(() => {
				requestAnimationFrame(() => {
					fitView({
						nodes: [{ id: node.id }],
						padding: 3,
						duration: 500,
					});
				});
			});
		},
		[
			board.data?.nodes,
			board.data?.layers,
			fitView,
			setCurrentLayer,
			setLayerPath,
		],
	);

	const pushLayer = useCallback(
		async (pushedLayer: ILayer) => {
			await saveViewport();

			setCurrentLayer(pushedLayer.id);
			setLayerPath((old) => {
				if (old) return `${old}/${pushedLayer.id}`;
				return pushedLayer.id;
			});
		},
		[saveViewport, setCurrentLayer, setLayerPath],
	);

	const popLayer = useCallback(() => {
		if (!layerPath) return;

		void saveViewport();

		const segments = layerPath.split("/");
		if (segments.length === 1) {
			setLayerPath(undefined);
			setCurrentLayer(undefined);
			return;
		}
		const newPath = segments.slice(0, -1).join("/");
		setLayerPath(newPath);
		const segment = newPath.split("/").pop();
		setCurrentLayer(segment);
	}, [layerPath, saveViewport, setCurrentLayer, setLayerPath]);

	return {
		focusNode,
		pushLayer,
		popLayer,
	};
}
