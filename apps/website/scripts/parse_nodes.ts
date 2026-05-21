import fs from "node:fs";
import type { IComment, INode, IPin, Node } from "@flow-like/flow-like-ui";
import { typeToColor } from "@flow-like/flow-like-ui/components/flow/utils";

const cache = new Map<string, [IPin, INode]>();

const nodeJson = JSON.parse(
	fs.readFileSync("./src/assets/site.json").toString(),
);

const parsedNodes: Node[] = [];
const parsedEdges: any[] = [];

const nodes: INode[] = Object.values(nodeJson.nodes);
const comments: IComment[] = Object.values(nodeJson.comments);

for (const node of nodes) {
	for (const pin of Object.values(node.pins)) {
		cache.set(pin.id, [pin, node]);
	}

	parsedNodes.push({
		id: node.id,
		type: "node",
		zIndex: 20,
		position: {
			x: node.coordinates?.[0] ?? 0,
			y: node.coordinates?.[1] ?? 0,
		},
		data: {
			label: node.name,
			node: node,
			boardId: "website",
			appId: "website",
			onExecute: async (node: INode, payload?: object) => {},
			onCopy: async () => {},
		},
	});
}

for (const [pin, node] of cache.values()) {
	if (pin.connected_to.length === 0) continue;

	for (const connectedTo of pin.connected_to) {
		const [conntectedPin, connectedNode] = cache.get(connectedTo) || [];
		if (!conntectedPin || !connectedNode) continue;

		const sourceNode = node.id;
		const connectedNodeId = connectedNode.id;
		if (pin.id && conntectedPin.id)
			parsedEdges.push({
				id: `${pin.id}-${conntectedPin.id}`,
				source: sourceNode,
				sourceHandle: pin.id,
				zIndex: 18,
				data: {
					fromLayer: node.layer,
					toLayer: connectedNode.layer,
					data_type: pin.data_type,
				},
				animated: pin.data_type !== "Execution",
				reconnectable: true,
				target: connectedNodeId,
				targetHandle: conntectedPin.id,
				style: { stroke: typeToColor(pin.data_type) },
				type: "default",
				data_type: pin.data_type,
			});
	}
}

for (const comment of comments) {
	parsedNodes.push({
		id: comment.id,
		type: "commentNode",
		position: { x: comment.coordinates[0], y: comment.coordinates[1] },
		width: comment.width ?? 200,
		height: comment.height ?? 80,
		zIndex: comment.z_index ?? 1,
		data: {
			label: comment.id,
			boardId: "website",
			comment: comment,
			onUpsert: (comment: IComment) => {},
		},
	});
}

fs.writeFileSync(
	"./public/board.json",
	JSON.stringify({
		nodes: parsedNodes,
		edges: parsedEdges,
	}),
);
