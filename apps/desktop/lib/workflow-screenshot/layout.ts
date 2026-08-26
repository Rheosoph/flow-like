import {
	type LayoutComment,
	type LayoutStyle,
	computeFlowLayoutDetailed,
} from "@flow-like/flow-like-ui/lib/flow-auto-layout";
import {
	type IBoard,
	type ILayer,
	ILayerType,
} from "@flow-like/flow-like-ui/lib/schema/flow/board";
import {
	type INode,
	type IPin,
	IPinType,
} from "@flow-like/flow-like-ui/lib/schema/flow/node";

function normalizedLayer(value: string | null | undefined): string | undefined {
	return value ? value : undefined;
}

function boundaryNode(
	layer: ILayer,
	suffix: "-input" | "-return",
	pins: Record<string, IPin>,
	coordinates: number[] | null | undefined,
	isStart: boolean,
): INode {
	return {
		id: `${layer.id}${suffix}`,
		category: "",
		coordinates: [coordinates?.[0] ?? 0, coordinates?.[1] ?? 0, 0],
		description: "",
		event_callback: false,
		friendly_name: layer.name,
		fn_refs: null,
		name: `${layer.id}${suffix}`,
		pins,
		start: isStart,
	};
}

function layerBoundaryNodes(layer: ILayer): INode[] {
	const inputPins: Record<string, IPin> = {};
	const returnPins: Record<string, IPin> = {};
	for (const pin of Object.values(layer.pins ?? {})) {
		const inverted: IPin = {
			...pin,
			pin_type:
				pin.pin_type === IPinType.Input ? IPinType.Output : IPinType.Input,
		};
		if (inverted.pin_type === IPinType.Output)
			inputPins[inverted.id] = inverted;
		else returnPins[inverted.id] = inverted;
	}

	const result: INode[] = [];
	if (Object.keys(inputPins).length > 0) {
		result.push(
			boundaryNode(layer, "-input", inputPins, layer.in_coordinates, true),
		);
	}
	if (Object.keys(returnPins).length > 0) {
		result.push(
			boundaryNode(layer, "-return", returnPins, layer.out_coordinates, false),
		);
	}
	return result;
}

function commentsForLayer(
	board: IBoard,
	currentLayer: string | undefined,
): LayoutComment[] {
	return Object.values(board.comments ?? {})
		.filter(
			(comment) =>
				normalizedLayer(comment.layer) === normalizedLayer(currentLayer),
		)
		.map((comment) => ({
			id: comment.id,
			x: comment.coordinates[0] ?? 0,
			y: comment.coordinates[1] ?? 0,
			width: comment.width ?? 200,
			height: comment.height ?? 200,
			isLocked: comment.is_locked === true,
		}));
}

/**
 * Apply the same pure layout engine used by FlowBoard to every canvas in a board. The interactive
 * app normally supplies measured DOM sizes; headless preparation deliberately uses the engine's
 * renderer-mirrored fallback measurements so the board is formatted before Chromium opens it.
 */
export function autoLayoutWorkflowBoard(
	board: IBoard,
	style: LayoutStyle,
): { canvases: number; positioned: number } {
	const canvases: Array<string | undefined> = [
		undefined,
		...Object.keys(board.layers ?? {}).sort(),
	];
	let laidOutCanvases = 0;
	let positioned = 0;

	for (const currentLayer of canvases) {
		const layerNodes = Object.values(board.nodes).filter(
			(node) => normalizedLayer(node.layer) === currentLayer,
		);
		const openLayer = currentLayer ? board.layers[currentLayer] : undefined;
		if (openLayer) layerNodes.push(...layerBoundaryNodes(openLayer));

		const layerEntities = Object.values(board.layers)
			.filter((layer) => {
				if (layer.type === ILayerType.Function && layer.id !== currentLayer) {
					return false;
				}
				return (
					normalizedLayer(layer.parent_id) === currentLayer &&
					layer.id !== currentLayer
				);
			})
			.map((layer) => ({ id: layer.id, coordinates: [...layer.coordinates] }));

		if (layerNodes.length === 0 && layerEntities.length === 0) continue;
		const comments = commentsForLayer(board, currentLayer);
		const { positions, commentPositions } = computeFlowLayoutDetailed(
			{
				layerNodes,
				layerEntities,
				boardLayers: board.layers,
				currentLayer,
				comments,
			},
			style,
		);

		for (const [id, [x, y]] of positions) {
			if (currentLayer && id === `${currentLayer}-input` && openLayer) {
				openLayer.in_coordinates = [x, y, 0];
				positioned += 1;
				continue;
			}
			if (currentLayer && id === `${currentLayer}-return` && openLayer) {
				openLayer.out_coordinates = [x, y, 0];
				positioned += 1;
				continue;
			}
			const node = board.nodes[id];
			if (node) {
				node.coordinates = [x, y, 0];
				positioned += 1;
				continue;
			}
			const layer = board.layers[id];
			if (layer) {
				layer.coordinates = [x, y, 0];
				positioned += 1;
			}
		}
		for (const [id, [x, y]] of commentPositions) {
			const comment = board.comments[id];
			if (!comment) continue;
			comment.coordinates = [x, y, 0];
		}
		laidOutCanvases += 1;
	}

	return { canvases: laidOutCanvases, positioned };
}
