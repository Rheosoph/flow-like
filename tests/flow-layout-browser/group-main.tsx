import { I18nProvider } from "@flow-like/locales";
import {
	Background,
	Controls,
	type Edge,
	Handle,
	type Node,
	type NodeProps,
	Position,
	ReactFlow,
	ReactFlowProvider,
	applyNodeChanges,
	useReactFlow,
} from "@xyflow/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { FlowDataEdge } from "../../packages/ui/components/flow/flow-data-edge";
import { GroupSuggestionsOverlay } from "../../packages/ui/components/flow/group-suggestions";
import { useGroupSuggestions } from "../../packages/ui/hooks/use-group-suggestions";
import { computeFlowLayoutDetailed } from "../../packages/ui/lib/flow-auto-layout";
import {
	type GroupSuggestion,
	evaluateGroupCandidate,
} from "../../packages/ui/lib/flow-grouping";
import {
	measureNodeBox,
	pinOffsetY,
} from "../../packages/ui/lib/flow-layout/measure";
import { GraphBuilder } from "../../packages/ui/lib/flow-layout/test-fixtures";
import type { IBoard } from "../../packages/ui/lib/schema/flow/board";
import {
	ICommandType,
	type IGenericCommand,
} from "../../packages/ui/lib/schema/flow/board/commands/generic-command";
import { type INode, IPinType } from "../../packages/ui/lib/schema/flow/node";
import "@xyflow/react/dist/style.css";
import "../../packages/ui/global.css";
import "./style.css";

function fixture(): IBoard {
	const graph = new GraphBuilder();
	for (const prefix of ["prompt", "response"]) {
		const y = prefix === "prompt" ? 0 : 420;
		graph.exec(`${prefix}-source`, {
			start: true,
			execIn: false,
			execOuts: 0,
			dataOuts: 1,
			coordinates: [0, y + 150, 0],
		});
		for (const [id, x, offsetY, dataIns] of [
			["a", 230, 150, 1],
			["b", 460, 40, 1],
			["c", 460, 270, 1],
			["d", 690, 150, 2],
			["sink", 920, 150, 1],
		] as const) {
			graph.pure(`${prefix}-${id}`, {
				dataIns,
				dataOuts: 1,
				coordinates: [x, y + offsetY, 0],
			});
		}
		for (const [from, to, index] of [
			["source", "a", 0],
			["a", "b", 0],
			["a", "c", 0],
			["b", "d", 0],
			["c", "d", 1],
			["d", "sink", 0],
		] as const)
			graph.dataLink(`${prefix}-${from}`, `${prefix}-${to}`, 0, index);
		const titles: Record<string, string> = {
			source: "Chat Event",
			a: "Prepare input",
			b: "Load context",
			c: "Apply policy",
			d: "Assemble message",
			sink: "Send response",
		};
		for (const [id, label] of Object.entries(titles)) {
			const node = graph.nodes.get(`${prefix}-${id}`);
			if (node) node.friendly_name = label;
		}
	}
	const nodes = Object.fromEntries(graph.nodes);
	const pins = new Map(
		Object.values(nodes).flatMap((node) =>
			Object.values(node.pins).map((pin) => [pin.id, pin] as const),
		),
	);
	for (const pin of pins.values())
		for (const target of pin.connected_to)
			pins.get(target)?.depends_on.push(pin.id);
	return {
		id: "group-browser-fixture",
		name: "Grouping",
		nodes,
		layers: {},
		comments: {},
		variables: {},
		refs: {},
	} as unknown as IBoard;
}

type FixtureNode = Node<{ node: INode }>;
function FixtureNodeView({ data }: NodeProps<FixtureNode>) {
	const node = data.node;
	return (
		<div className="fixture-node" style={measureNodeBox(node)}>
			<div className={node.start ? "fixture-title event" : "fixture-title"}>
				{node.friendly_name}
			</div>
			{Object.values(node.pins).map((pin) => {
				const input = pin.pin_type === IPinType.Input;
				const top = pinOffsetY(pin);
				return (
					<div key={pin.id}>
						<Handle
							id={pin.id}
							type={input ? "target" : "source"}
							position={input ? Position.Left : Position.Right}
							style={{
								top,
								width: 6,
								height: 6,
								border: "none",
								background: "#a20cff",
								[input ? "left" : "right"]: 0,
							}}
						/>
						<span
							className={`fixture-pin ${input ? "input" : "output"}`}
							style={{ top: top - 6 }}
						>
							{pin.friendly_name}
						</span>
					</div>
				);
			})}
		</div>
	);
}

const nodeTypes = { fixture: FixtureNodeView };
const edgeTypes = { data: FlowDataEdge };

function Fixture() {
	const flow = useReactFlow();
	const [board, setBoard] = useState(fixture);
	const [selected, setSelected] = useState<string[]>([]);
	const [readOnly, setReadOnly] = useState(false);
	const [revision, setRevision] = useState(0);
	const [positions, setPositions] = useState<
		Record<string, { x: number; y: number }>
	>({});
	const [measurements, setMeasurements] = useState<
		Record<string, { width: number; height: number }>
	>({});
	const [collapsed, setCollapsed] = useState<Record<string, GroupSuggestion>>(
		{},
	);
	const history = useRef<
		Array<{ board: IBoard; collapsed: Record<string, GroupSuggestion> }>
	>([]);
	const commands = useRef<IGenericCommand[]>([]);
	const errors = useRef<string[]>([]);
	const executeCommands = useCallback(
		async (batch: IGenericCommand[]) => {
			const command = batch[0];
			if (
				batch.length !== 1 ||
				command.command_type !== ICommandType.UpsertLayer ||
				!command.layer
			)
				throw new Error("Expected one collapsed-layer command");
			const candidate = evaluateGroupCandidate(
				{ board },
				command.node_ids ?? [],
			);
			if (!candidate) throw new Error("Invalid group membership");
			history.current.push({
				board: structuredClone(board),
				collapsed: structuredClone(collapsed),
			});
			commands.current = batch;
			const next = structuredClone(board);
			next.layers[command.layer.id] = command.layer;
			for (const id of command.node_ids ?? [])
				if (next.nodes[id]) next.nodes[id].layer = command.layer.id;
			setBoard(next);
			setCollapsed((previous) => ({
				...previous,
				[command.layer?.id ?? ""]: candidate,
			}));
			setRevision((value) => value + 1);
			return batch;
		},
		[board, collapsed],
	);
	const grouping = useGroupSuggestions({
		board,
		readOnly,
		selectedNodeIds: selected,
		getNodes: flow.getNodes,
		getInternalNode: flow.getInternalNode,
		executeCommands,
		onStale: () => errors.current.push("stale"),
	});
	const rendered = useMemo(() => {
		const nodes: FixtureNode[] = Object.values(board.nodes)
			.filter((node) => !node.layer)
			.map((node) => ({
				id: node.id,
				type: "fixture",
				data: { node },
				position: positions[node.id] ?? {
					x: node.coordinates?.[0] ?? 0,
					y: node.coordinates?.[1] ?? 0,
				},
				selected: selected.includes(node.id),
				...measureNodeBox(node),
				measured: measurements[node.id],
				style: grouping.previewMemberIds.has(node.id)
					? { opacity: 0.15 }
					: undefined,
			}));
		for (const [id, suggestion] of Object.entries(collapsed)) {
			const layer = board.layers[id];
			if (!layer) continue;
			let inputs = 0;
			let outputs = 0;
			const node = {
				id,
				name: "collapsed",
				friendly_name: layer.name,
				pins: Object.fromEntries(
					suggestion.boundaryPorts.map((port) => {
						const input = port.direction === "input";
						return [
							port.id,
							{
								id: port.id,
								name: port.label,
								friendly_name: port.label,
								pin_type: input ? IPinType.Input : IPinType.Output,
								index: input ? ++inputs : ++outputs,
								data_type: port.dataType,
								connected_to: [],
								depends_on: [],
							},
						];
					}),
				),
			} as unknown as INode;
			nodes.push({
				id,
				type: "fixture",
				data: { node },
				position: { x: layer.coordinates[0], y: layer.coordinates[1] },
				...measureNodeBox(node),
				measured: measurements[id],
			});
		}
		const owners = new Map(
			Object.values(board.nodes).flatMap((node) =>
				Object.keys(node.pins).map((pin) => [pin, node] as const),
			),
		);
		const edges: Edge[] = [];
		for (const node of Object.values(board.nodes))
			for (const pin of Object.values(node.pins))
				for (const target of pin.connected_to) {
					const to = owners.get(target);
					if (!to || (node.layer && node.layer === to.layer)) continue;
					const source = node.layer || node.id;
					const targetId = to.layer || to.id;
					const sourceHandle = node.layer ? `output:${pin.id}` : pin.id;
					const targetHandle = to.layer ? `input:${pin.id}` : target;
					edges.push({
						id: `${pin.id}->${target}`,
						source,
						target: targetId,
						sourceHandle,
						targetHandle,
						type: "data",
						className:
							grouping.previewMemberIds.has(source) ||
							grouping.previewMemberIds.has(targetId)
								? "opacity-10"
								: undefined,
						style: { stroke: "#a20cff" },
						data: { reduceMotion: true },
					});
				}
		return { nodes, edges };
	}, [
		board,
		collapsed,
		positions,
		selected,
		measurements,
		grouping.previewMemberIds,
	]);
	const layout = () => {
		grouping.close();
		const result = computeFlowLayoutDetailed(
			{
				layerNodes: Object.values(board.nodes).filter((node) => !node.layer),
				layerEntities: [],
				currentLayer: undefined,
			},
			"compact",
		);
		const next = structuredClone(board);
		for (const [id, point] of result.positions)
			next.nodes[id].coordinates = [...point, 0];
		setBoard(next);
		setPositions({});
		grouping.offerAfterLayout();
	};
	useEffect(() => {
		Object.assign(window, {
			groupQa: {
				snapshot: () => ({
					board,
					suggestions: grouping.suggestions,
					open: grouping.open,
					preview: grouping.preview,
					revision,
					commands: commands.current,
					errors: errors.current,
				}),
				fit: () => flow.fitView({ padding: 0.2, duration: 0 }),
				setReadOnly,
				setSelected,
				move: (id: string, x: number, y: number) =>
					setPositions((previous) => ({ ...previous, [id]: { x, y } })),
			},
		});
	}, [
		board,
		grouping.suggestions,
		grouping.open,
		grouping.preview,
		revision,
		flow,
	]);
	return (
		<main>
			<header>
				<h1>Group suggestions</h1>
				<button
					type="button"
					disabled={readOnly}
					onClick={() => grouping.start()}
				>
					Find groups
				</button>
				<button type="button" onClick={layout}>
					Auto Layout
				</button>
				<button
					type="button"
					disabled={!history.current.length}
					onClick={() => {
						const previous = history.current.pop();
						if (previous) {
							setBoard(previous.board);
							setCollapsed(previous.collapsed);
							setRevision((value) => value + 1);
						}
					}}
				>
					Undo
				</button>
				{grouping.offerCount > 0 && (
					<button type="button" onClick={() => grouping.start(false)}>
						Review {grouping.offerCount} grouping suggestions
					</button>
				)}
			</header>
			<div className="fixture-canvas">
				<ReactFlow
					nodes={rendered.nodes}
					edges={rendered.edges}
					nodeTypes={nodeTypes}
					edgeTypes={edgeTypes}
					onNodesChange={(changes) => {
						const next = applyNodeChanges(changes, rendered.nodes);
						if (changes.some((change) => change.type === "dimensions"))
							setMeasurements((previous) => {
								const updated = { ...previous };
								for (const change of changes)
									if (change.type === "dimensions" && change.dimensions)
										updated[change.id] = change.dimensions;
								return updated;
							});
						if (changes.some((change) => change.type === "select"))
							setSelected(
								next.filter((node) => node.selected).map((node) => node.id),
							);
						if (changes.some((change) => change.type === "position"))
							setPositions(
								Object.fromEntries(
									next.map((node) => [node.id, node.position]),
								),
							);
					}}
					fitView
					minZoom={0.2}
					maxZoom={3}
					colorMode="dark"
				>
					<Background gap={12} />
					<Controls />
					{grouping.open && (
						<GroupSuggestionsOverlay
							suggestions={grouping.suggestions}
							selectedId={grouping.selectedId}
							preview={grouping.preview}
							busy={grouping.busy}
							onSelect={grouping.select}
							onPreview={grouping.togglePreview}
							onCollapse={() => void grouping.collapse()}
							onDismiss={grouping.dismiss}
							onClose={grouping.close}
							getPinPosition={grouping.getPinPosition}
						/>
					)}
				</ReactFlow>
			</div>
		</main>
	);
}

createRoot(document.getElementById("root") as HTMLElement).render(
	<I18nProvider>
		<ReactFlowProvider>
			<Fixture />
		</ReactFlowProvider>
	</I18nProvider>,
);
