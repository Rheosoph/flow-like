import { Background, BackgroundVariant, ReactFlow } from "@flow-like/flow-like-ui";
import { CommentNode } from "@flow-like/flow-like-ui/components/flow/comment-node";
import { FlowNode } from "@flow-like/flow-like-ui/components/flow/flow-node";
import { LayerNode } from "@flow-like/flow-like-ui/components/flow/layer-node";
import { useEffect, useMemo, useState } from "react";

export default function Board({
	nodes,
	edges,
}: Readonly<{ nodes: any[]; edges: any[] }>) {
	const nodeTypes = useMemo(
		() => ({
			flowNode: FlowNode,
			commentNode: CommentNode,
			layerNode: LayerNode,
			node: FlowNode,
		}),
		[],
	);

	const [isMobile, setIsMobile] = useState(window.innerWidth <= 768);
	useEffect(() => {
		const update = () => setIsMobile(window.innerWidth <= 768);
		update();
		window.addEventListener("resize", update);
		return () => window.removeEventListener("resize", update);
	}, []);

	return (
		<ReactFlow
			className="absolute top-0 left-0 right-0 bottom-0"
			suppressHydrationWarning
			nodesDraggable={false}
			nodesConnectable={false}
			colorMode={"dark"}
			nodes={nodes}
			nodeTypes={nodeTypes}
			edges={edges}
			maxZoom={3}
			minZoom={0.1}
			onInit={(instance) => {
				instance.fitView({
					nodes: isMobile
						? [
								{
									id: "j7a5erre9fwqhbq1k5f27tma",
								},
								{
									id: "hh8j23jlgf45jwqb2iwxzkt5",
								},
							]
						: [
								{
									id: "j7a5erre9fwqhbq1k5f27tma",
								},
								{
									id: "hh8j23jlgf45jwqb2iwxzkt5",
								},
							],
					padding: 0.25,
				});
			}}
			proOptions={{ hideAttribution: true }}
		>
			<Background variant={BackgroundVariant.Dots} gap={12} size={1} />
		</ReactFlow>
	);
}
