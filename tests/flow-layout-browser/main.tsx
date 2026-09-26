import { I18nProvider } from "@flow-like/locales";
import {
	Background,
	type Edge,
	Handle,
	type Node,
	type NodeProps,
	Position,
	ReactFlow,
	ReactFlowProvider,
	useNodesInitialized,
	useReactFlow,
} from "@xyflow/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { AutoLayoutDialog } from "../../packages/ui/components/flow/auto-layout-dialog";
import { FlowDataEdge } from "../../packages/ui/components/flow/flow-data-edge";
import { FlowExecutionEdge } from "../../packages/ui/components/flow/flow-execution-edge";
import {
	type LayoutStyle,
	computeFlowLayoutDetailed,
} from "../../packages/ui/lib/flow-auto-layout";
import {
	measureNodeBox,
	pinOffsetY,
} from "../../packages/ui/lib/flow-layout/measure";
import { buildAutoRerouteCommands } from "../../packages/ui/lib/flow-layout/reroute-commands";
import { GraphBuilder } from "../../packages/ui/lib/flow-layout/test-fixtures";
import {
	ICommandType,
	type IGenericCommand,
} from "../../packages/ui/lib/schema/flow/board/commands/generic-command";
import {
	type INode,
	IPinType,
	IVariableType,
} from "../../packages/ui/lib/schema/flow/node";
import "@xyflow/react/dist/style.css";
import "../../packages/ui/global.css";
import "./style.css";

function required<T>(value: T | null | undefined): T {
	if (value === undefined || value === null)
		throw new Error("Missing fixture value");
	return value;
}

function fixture(name: string): INode[] {
	const graph = new GraphBuilder();
	if (name === "crossing") {
		for (const id of ["a", "b", "c", "d"]) graph.pure(id);
		graph.dataLink("a", "b");
		graph.dataLink("c", "d");
	} else if (name === "parallel" || name === "parallel-swap") {
		const swap = name === "parallel-swap";
		graph.exec("fork", {
			start: true,
			execIn: false,
			execOuts: ["upper", "lower"],
		});
		graph.exec("join", { execOuts: 0 });
		for (const branch of ["upper", "lower"]) {
			const heights = branch === "upper" ? [0, 8, 4, 2] : [0, 4, 7, 2];
			for (let index = 0; index < 4; index++) {
				graph.exec(`${branch}-${index}`, {
					dataIns: swap ? 6 : heights[index],
					dataOuts: swap ? 6 : index === 0 ? 2 : 0,
				});
				if (index > 0)
					graph.execLink(`${branch}-${index - 1}`, `${branch}-${index}`);
			}
			graph.execLink("fork", `${branch}-0`, branch);
			graph.execLink(`${branch}-3`, "join");
			if (!swap) {
				graph.dataLink(`${branch}-0`, `${branch}-3`, 0, 0);
				graph.dataLink(`${branch}-0`, `${branch}-3`, 1, 1);
			}
		}
		if (swap) {
			graph.dataLink("upper-0", "upper-2", 2, 4);
			graph.dataLink("lower-0", "lower-2", 3, 4);
			graph.dataLink("lower-1", "lower-3", 5, 2);
			graph.dataLink("lower-1", "lower-3", 0, 4);
			graph.dataLink("lower-1", "lower-3", 3, 5);
		}
	} else if (name === "dense") {
		for (let index = 0; index < 7; index++) {
			graph.exec(`step-${index}`, {
				start: index === 0,
				execIn: index !== 0,
				dataIns: 4,
				dataOuts: 4,
			});
			if (index > 0) graph.execLink(`step-${index - 1}`, `step-${index}`);
		}
		graph.dataLink("step-0", "step-5", 0, 2);
		graph.dataLink("step-0", "step-5", 1, 1);
		graph.dataLink("step-0", "step-6", 2, 0);
		graph.dataLink("step-1", "step-6", 0, 2);
		graph.dataLink("step-2", "step-6", 1, 1);
		graph.dataLink("step-1", "step-4", 1, 0);
		graph.dataLink("step-2", "step-5", 0, 0);
	} else {
		graph.exec("chat", { start: true, execIn: false, dataOuts: 7 });
		graph.exec("system", { dataIns: 2, dataOuts: 1 });
		graph.exec("model", { dataIns: 1, dataOuts: 1 });
		graph.exec("invoke", { dataIns: 2, dataOuts: 4 });
		graph.execLink("chat", "system");
		graph.execLink("system", "model");
		graph.execLink("model", "invoke");
		graph.dataLink("chat", "system");
		graph.dataLink("model", "invoke");
		graph.dataLink("system", "invoke", 0, 1);
		required(graph.nodes.get("chat")).friendly_name = "Chat Event";
		required(graph.nodes.get("system")).friendly_name = "Set System Message";
		required(graph.nodes.get("model")).friendly_name = "Find Model";
		required(graph.nodes.get("invoke")).friendly_name = "Invoke Model";
	}
	const input = graph.build();
	const pins = new Map(
		input.layerNodes.flatMap((node) =>
			Object.values(node.pins).map((pin) => [pin.id, pin] as const),
		),
	);
	for (const pin of pins.values()) {
		for (const target of pin.connected_to)
			pins.get(target)?.depends_on.push(pin.id);
	}
	if (name === "crossing") {
		const coordinates: Record<string, number[]> = {
			a: [0, 62, 0],
			b: [800, 62, 0],
			c: [230, -28, 0],
			d: [460, 122, 0],
		};
		return input.layerNodes.map((node) => ({
			...node,
			coordinates: coordinates[node.id],
		}));
	}
	const result = computeFlowLayoutDetailed(input, "compact");
	return input.layerNodes.map((node) => ({
		...node,
		coordinates: [...required(result.positions.get(node.id)), 0],
	}));
}

function rerouteTemplate(): INode {
	const graph = new GraphBuilder();
	const node = graph.pure("reroute-template");
	node.name = "reroute";
	for (const pin of Object.values(node.pins)) {
		pin.name = pin.pin_type === IPinType.Input ? "route_in" : "route_out";
	}
	return node;
}

function applyCommands(nodes: INode[], commands: IGenericCommand[]): INode[] {
	const next = new Map(structuredClone(nodes).map((node) => [node.id, node]));
	for (const command of commands) {
		if (command.command_type === ICommandType.AddNode && command.node) {
			next.set(command.node.id, structuredClone(command.node));
		} else if (command.command_type === ICommandType.RemoveNode) {
			const removedId = command.node?.id ?? required(command.node_id);
			const removed = next.get(removedId);
			const pins = new Set(Object.keys(removed?.pins ?? {}));
			next.delete(removedId);
			for (const node of next.values()) {
				for (const pin of Object.values(node.pins)) {
					pin.connected_to = pin.connected_to.filter((id) => !pins.has(id));
					pin.depends_on = pin.depends_on.filter((id) => !pins.has(id));
				}
			}
		} else if (command.command_type === ICommandType.MoveNode) {
			const node = next.get(required(command.node_id));
			if (node) node.coordinates = command.to_coordinates;
		} else if (
			command.command_type === ICommandType.ConnectPin ||
			command.command_type === ICommandType.DisconnectPin
		) {
			const source = next.get(required(command.from_node))?.pins[
				required(command.from_pin)
			];
			const target = next.get(required(command.to_node))?.pins[
				required(command.to_pin)
			];
			if (!source || !target)
				throw new Error("Command references a missing pin");
			source.connected_to = source.connected_to.filter(
				(id) => id !== target.id,
			);
			target.depends_on = target.depends_on.filter((id) => id !== source.id);
			if (command.command_type === ICommandType.ConnectPin) {
				source.connected_to.push(target.id);
				target.depends_on.push(source.id);
			}
		} else {
			throw new Error(`Unsupported fixture command: ${command.command_type}`);
		}
	}
	return [...next.values()];
}

type FixtureNode = Node<{ node: INode }>;
function BoardNode({ data }: NodeProps<FixtureNode>) {
	const node = data.node;
	const reroute = node.name === "reroute";
	const size = measureNodeBox(node);
	return (
		<div className={reroute ? "fixture-reroute" : "fixture-node"} style={size}>
			{!reroute && (
				<div className={node.start ? "fixture-title event" : "fixture-title"}>
					{node.friendly_name}
				</div>
			)}
			{Object.values(node.pins).map((pin) => {
				const input = pin.pin_type === IPinType.Input;
				const execution = pin.data_type === IVariableType.Execution;
				const y = pinOffsetY(pin, reroute);
				return (
					<div key={pin.id}>
						<Handle
							id={pin.id}
							type={input ? "target" : "source"}
							position={input ? Position.Left : Position.Right}
							style={{
								top: y,
								width: reroute ? 8 : 6,
								height: reroute ? 8 : 6,
								border: "none",
								background: execution ? "#eee" : "#a20cff",
								[input ? "left" : "right"]: 0,
							}}
						/>
						{!reroute && (
							<span
								className={input ? "fixture-pin input" : "fixture-pin output"}
								style={{ top: y - 6 }}
							>
								{execution ? "◆" : pin.friendly_name}
							</span>
						)}
					</div>
				);
			})}
		</div>
	);
}
const nodeTypes = { fixture: BoardNode };
const edgeTypes = { data: FlowDataEdge, execution: FlowExecutionEdge };

function BoardFixture() {
	const requestedPath = new URLSearchParams(location.search).get("path");
	const pathType =
		requestedPath === "straight" ||
		requestedPath === "step" ||
		requestedPath === "smoothstep"
			? requestedPath
			: "default";
	const scenario =
		new URLSearchParams(location.search).get("scenario") ?? "screenshot";
	const [board, setBoard] = useState(() => fixture(scenario));
	const [open, setOpen] = useState(false);
	const [style, setStyle] = useState<LayoutStyle>("compact");
	const [revision, setRevision] = useState(0);
	const history = useRef<INode[][]>([]);
	const lastCommands = useRef<IGenericCommand[]>([]);
	const flow = useReactFlow();
	const initialized = useNodesInitialized();
	const { nodes, edges } = useMemo(() => {
		const owners = new Map(
			board.flatMap((node) =>
				Object.keys(node.pins).map((pin) => [pin, node.id] as const),
			),
		);
		const edges: Edge[] = [];
		for (const node of board) {
			for (const pin of Object.values(node.pins)) {
				if (pin.pin_type !== IPinType.Output) continue;
				for (const target of pin.connected_to) {
					const targetNode = owners.get(target);
					if (!targetNode) continue;
					edges.push({
						id: `${pin.id}:${target}`,
						source: node.id,
						target: targetNode,
						sourceHandle: pin.id,
						targetHandle: target,
						type:
							pin.data_type === IVariableType.Execution ? "execution" : "data",
						style: {
							stroke:
								pin.data_type === IVariableType.Execution
									? "#d1d1d1"
									: "#a20cff",
						},
						data: { pathType, reduceMotion: true },
					});
				}
			}
		}
		return {
			nodes: board.map((node) => ({
				id: node.id,
				type: "fixture",
				position: {
					x: node.coordinates?.[0] ?? 0,
					y: node.coordinates?.[1] ?? 0,
				},
				data: { node },
				...measureNodeBox(node),
			})),
			edges,
		};
	}, [board, pathType]);
	useEffect(() => {
		if (!initialized) return;
		const frame = requestAnimationFrame(() => {
			void flow.fitView({ padding: 0.2, duration: 0, nodes });
		});
		return () => cancelAnimationFrame(frame);
	}, [flow, initialized, nodes]);
	const applyLayout = useCallback(
		(selectedStyle: LayoutStyle) => {
			const offsets = new Map<string, { x: number; y: number }>();
			const sizes = new Map(
				board.map((node) => {
					const internal = flow.getInternalNode(node.id);
					for (const handle of [
						...(internal?.internals.handleBounds?.source ?? []),
						...(internal?.internals.handleBounds?.target ?? []),
					]) {
						if (handle.id)
							offsets.set(handle.id, {
								x: handle.x + handle.width / 2,
								y: handle.y + handle.height / 2,
							});
					}
					const box = measureNodeBox(node);
					return [
						node.id,
						[
							internal?.measured.width ?? box.width,
							internal?.measured.height ?? box.height,
						] as const,
					] as const;
				}),
			);
			const result = computeFlowLayoutDetailed(
				{
					layerNodes: board,
					layerEntities: [],
					currentLayer: undefined,
					nodeSizes: sizes,
					pinOffsets: offsets,
					edgePathType: pathType,
				},
				selectedStyle,
			);
			const cache = new Map(
				board.flatMap((node) =>
					Object.values(node.pins).map(
						(pin) =>
							[
								pin.id,
								[pin, node, false] as [typeof pin, INode, boolean],
							] as const,
					),
				),
			);
			const commands: IGenericCommand[] = [];
			for (const [id, position] of result.positions) {
				commands.push({
					command_type: ICommandType.MoveNode,
					node_id: id,
					to_coordinates: [...position, 0],
				});
			}
			if (result.routing)
				commands.push(
					...buildAutoRerouteCommands({
						routes: result.routing.routes,
						chains: result.routing.chains,
						reroute: rerouteTemplate(),
						currentLayer: undefined,
						pinCache: cache,
					}),
				);
			history.current.push(structuredClone(board));
			lastCommands.current = commands;
			setBoard(applyCommands(board, commands));
			setStyle(selectedStyle);
			setRevision((current) => current + 1);
		},
		[board, flow, pathType],
	);
	useEffect(() => {
		Object.assign(window, {
			layoutQa: {
				snapshot: () => ({
					board,
					edges,
					revision,
					style,
					commands: lastCommands.current,
				}),
				applyLayout,
				fit: () => flow.fitView({ padding: 0.15, duration: 0 }),
				measurements: () =>
					board
						.filter((node) => !node.auto_reroute)
						.map((node) => {
							const internal = flow.getInternalNode(node.id);
							return {
								id: node.id,
								size: internal?.measured,
								handles: internal?.internals.handleBounds,
							};
						}),
			},
		});
	}, [board, edges, revision, style, applyLayout, flow]);
	return (
		<main>
			<header>
				<h1>Automatic data routing</h1>
				<button type="button" onClick={() => setOpen(true)}>
					Auto Layout
				</button>
				<button type="button" onClick={() => applyLayout(style)}>
					Repeat layout
				</button>
				<button
					type="button"
					disabled={history.current.length === 0}
					onClick={() => {
						const previous = history.current.pop();
						if (previous) {
							setBoard(previous);
							setRevision((current) => current + 1);
						}
					}}
				>
					Undo
				</button>
				<output>
					{scenario} · {style} ·{" "}
					{board.filter((node) => node.auto_reroute).length} reroutes
				</output>
			</header>
			<div className="fixture-canvas">
				<ReactFlow
					nodes={nodes}
					edges={edges}
					nodeTypes={nodeTypes}
					edgeTypes={edgeTypes}
					nodesDraggable={false}
					nodesConnectable={false}
					minZoom={0.1}
					maxZoom={2}
					fitView
				>
					<Background gap={12} size={0.6} />
				</ReactFlow>
			</div>
			<AutoLayoutDialog
				open={open}
				onOpenChange={setOpen}
				onSelect={applyLayout}
			/>
		</main>
	);
}

createRoot(required(document.getElementById("root"))).render(
	<I18nProvider language="en">
		<ReactFlowProvider>
			<BoardFixture />
		</ReactFlowProvider>
	</I18nProvider>,
);
