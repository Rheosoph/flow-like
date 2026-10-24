import {
	type Connection,
	type Edge,
	type Node,
	type NodeProps,
	Position,
	applyEdgeChanges,
	applyNodeChanges,
} from "@xyflow/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import {
	BoardRuntimeProvider,
	useReactFlow,
} from "../../packages/ui/components/flow/board-runtime";
import { Handle } from "../../packages/ui/components/flow/board-runtime-primitives";
import { EguiFlowCanvas } from "../../packages/ui/components/flow/egui-flow-canvas";
import { prepareNativeSceneNodes } from "../../packages/ui/components/flow/egui-flow-proxy";
import type { FlowCanvasProps } from "../../packages/ui/components/flow/flow-board";
import { FlowDataEdge } from "../../packages/ui/components/flow/flow-data-edge";
import { FlowExecutionEdge } from "../../packages/ui/components/flow/flow-execution-edge";
import {
	FlowHelperLinesLayer,
	useHelperLines,
} from "../../packages/ui/components/flow/flow-helper-lines";
import {
	buildConnectionCommands,
	buildDisconnectionCommands,
} from "../../packages/ui/lib/board-canvas/connection-commands";
import {
	type Palette,
	nodeNeedsDom,
	sceneFromFlow,
} from "../../packages/ui/lib/board-canvas/scene";
import { updateNodeCommand } from "../../packages/ui/lib/command/generic-command";
import { isValidConnection } from "../../packages/ui/lib/flow-board-utils";
import type { IGenericCommand } from "../../packages/ui/lib/schema/flow/board/commands/generic-command";
import type { INode } from "../../packages/ui/lib/schema/flow/node";
import { useRunExecutionStore } from "../../packages/ui/state/run-execution-state";
import { applyCommands, inverse, makeBoard, pinCache, project } from "./model";
import {
	FixtureRendererSettings,
	initializeFixtureRenderer,
} from "./renderer-settings";
import "@xyflow/react/dist/style.css";
import "../../packages/ui/global.css";
import "./style.css";

const layoutPalette: Palette = {
	background: [0, 0, 0, 255],
	card: [20, 20, 20, 255],
	foreground: [240, 240, 240, 255],
	muted: [100, 100, 100, 255],
	border: [80, 80, 80, 255],
	primary: [250, 100, 60, 255],
	tertiary: [170, 100, 200, 255],
};
const count = Math.max(
	10,
	Math.min(
		10000,
		Number(new URLSearchParams(location.search).get("nodes")) || 10,
	),
);
const richCount = Math.max(
	0,
	Math.min(
		count,
		Number(new URLSearchParams(location.search).get("richNodes")) || 0,
	),
);
function makeFixtureBoard() {
	const board = makeBoard(count);
	// The farthest nodes are rich, so culling tests scroll an editor into view.
	for (const node of richCount ? Object.values(board).slice(-richCount) : [])
		node.comment = "Rich-editor culling fixture";
	return board;
}
const pause = (ms: number) =>
	new Promise<void>((resolve) => setTimeout(resolve, ms));
const ACTIVITY_RUN_ID = "native-canvas-harness-activity";

/** Only the rich node editor is replaced here; canvas, gestures, validation and transactions are production code. */
function HarnessNode(props: NodeProps) {
	const [clicked, setClicked] = useState(0);
	const [pinEditorClicks, setPinEditorClicks] = useState(0);
	const shape = sceneFromFlow(
		[
			{
				id: props.id,
				type: "node",
				position: { x: 0, y: 0 },
				data: props.data,
			},
		],
		[],
		layoutPalette,
	).nodes[0];
	const node = props.data.node as INode;
	const header =
		shape.kind === "pure"
			? "var(--tertiary)"
			: shape.kind === "event"
				? "var(--primary)"
				: shape.kind === "function"
					? "#8b5cf6"
					: "var(--card)";
	return (
		<div
			className={`harness-node ${props.selected ? "selected" : ""}`}
			style={{ width: shape.width, height: shape.height }}
		>
			<div
				className="harness-node-title"
				style={{ background: `linear-gradient(90deg,var(--card),${header})` }}
			>
				{shape.label}
			</div>
			{shape.pins.map((pin) => (
				<div key={pin.id}>
					<Handle
						id={pin.id}
						type={pin.direction === "input" ? "target" : "source"}
						position={
							pin.kind === "reference"
								? pin.direction === "input"
									? Position.Top
									: Position.Bottom
								: pin.direction === "input"
									? Position.Left
									: Position.Right
						}
						style={{
							left: pin.x,
							top: pin.y,
							right: "auto",
							bottom: "auto",
							transform: "translate(-50%, -50%)",
							width: 12,
							height: 12,
							border: "none",
							background: "transparent",
						}}
					>
						<span
							style={{
								pointerEvents: "none",
								position: "absolute",
								left: 2.5,
								top: 2.5,
								width: 7,
								height: 7,
								borderRadius: pin.kind === "execution" ? 0 : "50%",
								transform:
									pin.kind === "execution" ? "rotate(45deg)" : undefined,
								background:
									pin.kind === "execution"
										? "var(--foreground)"
										: pin.kind === "reference"
											? "var(--pin-fn-ref,var(--tertiary))"
											: `var(--pin-${node.pins[pin.id]?.data_type.toLowerCase()},var(--foreground))`,
							}}
						/>
						{props.selected &&
							pin ===
								shape.pins.find((entry) => entry.direction === "input") && (
								<div
									data-board-dom-control
									data-testid="harness-pin-editor"
									// biome-ignore lint/a11y/useSemanticElements: This fixture reproduces the production div editor nested inside a handle.
									role="button"
									tabIndex={0}
									onClick={(event) => {
										event.stopPropagation();
										setPinEditorClicks((value) => value + 1);
									}}
									onKeyDown={(event) => {
										if (event.key === "Enter" || event.key === " ") {
											event.preventDefault();
											setPinEditorClicks((value) => value + 1);
										}
									}}
									onPointerDown={(event) => event.stopPropagation()}
									onMouseDown={(event) => event.stopPropagation()}
									style={{
										position: "absolute",
										left: 24,
										top: -2,
										width: 80,
										height: 18,
										fontSize: 10,
										background: "var(--card)",
										pointerEvents: "all",
									}}
								>
									Pin editor {pinEditorClicks}
								</div>
							)}
					</Handle>
					{pin.label && (
						<span
							className="harness-pin-label"
							style={{
								top: pin.y,
								...(pin.direction === "input" ? { left: 9 } : { right: 9 }),
							}}
						>
							{pin.label}
						</span>
					)}
				</div>
			))}
			{props.selected && (
				<button
					type="button"
					className="nodrag nopan"
					onClick={() => setClicked((n) => n + 1)}
				>
					DOM control {clicked}
				</button>
			)}
		</div>
	);
}
const nodeTypes = { node: HarnessNode };
const edgeTypes = { data: FlowDataEdge, execution: FlowExecutionEdge };

function Harness() {
	const flow = useReactFlow();
	const helper = useHelperLines();
	const [initial] = useState(makeFixtureBoard);
	const board = useRef(initial);
	const [first] = useState(() => project(initial));
	const [nodes, setNodes] = useState<Node[]>(first.nodes);
	const [edges, setEdges] = useState<Edge[]>(first.edges);
	const current = useRef({ nodes, edges });
	current.current = { nodes, edges };
	const history = useRef<{
		undo: IGenericCommand[][];
		redo: IGenericCommand[][];
	}>({ undo: [], redo: [] });
	const commits = useRef(0);
	const nativeConnections = useRef(0);
	const legacyConnections = useRef(0);
	const cancelledDrags = useRef(0);
	const screenshot = useRef<
		| {
				nodes: number;
				edges: number;
				nativeCanvases: number;
				asyncComplete: boolean;
		  }
		| undefined
	>(undefined);
	const paletteDrops = useRef(0);
	const delay = useRef(0);
	const pending = useRef(false);
	const epoch = useRef(0);
	const [version, setVersion] = useState(0);
	const [readOnly, setReadOnly] = useState(false);
	const locked = useRef(readOnly);
	locked.current = readOnly;
	const [scope, setScope] = useState(0);
	const [message, setMessage] = useState("");
	const publish = useCallback(() => {
		const result = project(
			board.current,
			current.current.nodes,
			current.current.edges,
		);
		setNodes(result.nodes);
		setEdges(result.edges);
		setVersion((n) => n + 1);
	}, []);
	const execute = useCallback(
		async (commands: IGenericCommand[] | undefined) => {
			if (!commands)
				throw new Error("Connection rejected by production validation");
			if (!commands.length) return;
			if (locked.current || pending.current)
				throw new Error("The simulated backend is locked or busy");
			pending.current = true;
			const scopeEpoch = epoch.current;
			setVersion((n) => n + 1);
			try {
				if (delay.current) await pause(delay.current);
				if (epoch.current !== scopeEpoch || locked.current) return;
				const next = structuredClone(board.current);
				const receipts = applyCommands(next, commands);
				board.current = next;
				history.current.undo.push(receipts);
				history.current.redo = [];
				commits.current++;
				setMessage(
					`${commands.length} command${commands.length === 1 ? "" : "s"} committed atomically`,
				);
				publish();
			} finally {
				pending.current = false;
				setVersion((n) => n + 1);
			}
		},
		[publish],
	);
	const connect = useCallback(
		async (connection: Connection, oldEdge?: Edge) =>
			execute(
				buildConnectionCommands({
					connection,
					oldEdge,
					boardNodes: board.current,
					pinCache: pinCache(board.current),
					refs: {},
				}),
			),
		[execute],
	);
	const nativeConnect = useCallback(
		async (connection: Connection, oldEdge?: Edge) => {
			nativeConnections.current++;
			return connect(connection, oldEdge);
		},
		[connect],
	);
	const legacyConnect = useCallback(
		(connection: Connection) => {
			legacyConnections.current++;
			void connect(connection).catch((error) => setMessage(String(error)));
		},
		[connect],
	);
	const legacyReconnect = useCallback(
		(edge: Edge, connection: Connection) => {
			legacyConnections.current++;
			void connect(connection, edge).catch((error) =>
				setMessage(String(error)),
			);
		},
		[connect],
	);
	const cancelDrag = useCallback(() => {
		cancelledDrags.current++;
		helper.end();
	}, [helper]);
	const disconnect = useCallback(
		async (edge: Edge) =>
			execute(
				buildDisconnectionCommands({
					edge,
					boardNodes: board.current,
					pinCache: pinCache(board.current),
				}),
			),
		[execute],
	);
	const undo = useCallback(() => {
		if (pending.current || locked.current) return;
		const receipts = history.current.undo.pop();
		if (!receipts) return;
		applyCommands(board.current, inverse(receipts));
		history.current.redo.push(receipts);
		publish();
	}, [publish]);
	const redo = useCallback(() => {
		if (pending.current || locked.current) return;
		const commands = history.current.redo.pop();
		if (!commands) return;
		history.current.undo.push(applyCommands(board.current, commands));
		publish();
	}, [publish]);
	const reset = useCallback(async () => {
		epoch.current++;
		useRunExecutionStore.getState().removeRun(ACTIVITY_RUN_ID);
		board.current = makeFixtureBoard();
		history.current = { undo: [], redo: [] };
		commits.current = 0;
		nativeConnections.current = 0;
		legacyConnections.current = 0;
		cancelledDrags.current = 0;
		paletteDrops.current = 0;
		delay.current = 0;
		locked.current = false;
		setReadOnly(false);
		setMessage("");
		setNodes([]);
		setEdges([]);
		const result = project(board.current);
		setNodes(result.nodes);
		setEdges(result.edges);
		setVersion((n) => n + 1);
		await flow.setViewport({ x: 20, y: 10, zoom: 1 });
		await pause(70);
	}, [flow]);
	const reject = useCallback(
		(error: unknown) =>
			setMessage(error instanceof Error ? error.message : String(error)),
		[],
	);
	const valid = useCallback(
		(connection: Connection | Edge) =>
			!locked.current &&
			isValidConnection(connection, pinCache(board.current), {}),
		[],
	);
	const onNodesChange = useCallback<
		NonNullable<FlowCanvasProps["onNodesChange"]>
	>(
		(changes) => {
			helper.snap(changes);
			setNodes((nodes) => applyNodeChanges(changes, nodes));
		},
		[helper],
	);
	const onEdgesChange = useCallback<
		NonNullable<FlowCanvasProps["onEdgesChange"]>
	>((changes) => setEdges((edges) => applyEdgeChanges(changes, edges)), []);
	const onNodeDragStop = useCallback<
		NonNullable<FlowCanvasProps["onNodeDragStop"]>
	>(
		(_event, node, dragged) => {
			helper.end();
			void execute(
				(dragged?.length ? dragged : [node]).map((moved) =>
					updateNodeCommand({
						node: {
							...board.current[moved.id],
							coordinates: [moved.position.x, moved.position.y, 0],
						},
					}),
				),
			).catch(reject);
		},
		[execute, reject, helper],
	);
	const onNodeDragStart = useCallback<
		NonNullable<FlowCanvasProps["onNodeDragStart"]>
	>(
		(event, node, dragged) =>
			helper.begin(event, dragged?.length ? dragged : [node]),
		[helper],
	);
	const onConnectEnd = useCallback<
		NonNullable<FlowCanvasProps["onConnectEnd"]>
	>((_event, state) => {
		if (!state.isValid && state.fromHandle) {
			paletteDrops.current++;
			setMessage(
				"Empty drop: host palette requested (palette content is outside this harness).",
			);
			setVersion((n) => n + 1);
		}
	}, []);
	// Every renderer calls onInit when it mounts, so a renderer switch would reset the camera.
	const cameraPlaced = useRef(false);
	const onInit = useCallback<NonNullable<FlowCanvasProps["onInit"]>>(
		(instance) => {
			if (count !== 10 || cameraPlaced.current) return;
			cameraPlaced.current = true;
			setTimeout(
				() => void instance.setViewport({ x: 20, y: 10, zoom: 1 }),
				80,
			);
		},
		[],
	);
	const noop = useCallback(() => {}, []);
	const capture = useCallback(async () => {
		screenshot.current = {
			nodes: document.querySelectorAll(".harness-node").length,
			edges: document.querySelectorAll(".react-flow__edge-path").length,
			nativeCanvases: document.querySelectorAll(
				"[data-testid='egui-board-canvas']",
			).length,
			asyncComplete: false,
		};
		await pause(120);
		screenshot.current.asyncComplete = true;
	}, []);
	const mapColor = useCallback(() => "var(--primary)", []);
	useEffect(() => {
		const scene = () =>
			sceneFromFlow(
				prepareNativeSceneNodes(current.current.nodes),
				current.current.edges,
				layoutPalette,
			);
		const client = (point: { x: number; y: number }) => {
			const element = document.querySelector(".react-flow");
			if (!element) throw new Error("Canvas has not mounted");
			const rect = element.getBoundingClientRect();
			const view = flow.getViewport();
			return {
				x: rect.left + view.x + point.x * view.zoom,
				y: rect.top + view.y + point.y * view.zoom,
			};
		};
		const pin = (nodeId: string, pinId: string) => {
			const n = scene().nodes.find((n) => n.id === nodeId);
			const p = n?.pins.find((p) => p.id === pinId);
			if (!n || !p) throw new Error(`Unknown pin ${pinId}`);
			return client({ x: n.x + p.x, y: n.y + p.y });
		};
		const api = {
			ready: () =>
				document
					.querySelector("[data-board-renderer]")
					?.getAttribute("data-board-renderer") === "egui",
			snapshot: () => ({
				nodes: structuredClone(board.current),
				edges: structuredClone(current.current.edges),
				history: {
					undo: history.current.undo.length,
					redo: history.current.redo.length,
				},
				commits: commits.current,
				nativeConnections: nativeConnections.current,
				legacyConnections: legacyConnections.current,
				cancelledDrags: cancelledDrags.current,
				screenshot: screenshot.current,
				paletteDrops: paletteDrops.current,
				pending: pending.current,
				readOnly: locked.current,
				selectedNodes: current.current.nodes
					.filter((n) => n.selected)
					.map((n) => n.id),
				selectedEdges: current.current.edges
					.filter((e) => e.selected)
					.map((e) => e.id),
				viewport: flow.getViewport(),
				presentation: {
					nativeNodes: flow
						.getNodes()
						.filter((node) => node.data.__flowNativeDom === false).length,
					domNodes: flow
						.getNodes()
						.filter((node) => node.data.__flowNativeDom !== false).length,
					nativeEdges: flow
						.getEdges()
						.filter((edge) => edge.data?.__flowNativeDom === false).length,
					domEdges: flow
						.getEdges()
						.filter((edge) => edge.data?.__flowNativeDom !== false).length,
					activeNodes:
						useRunExecutionStore.getState().runs.get(ACTIVITY_RUN_ID)?.nodes
							.size ?? 0,
				},
			}),
			pin,
			node: (id: string) => {
				const n = scene().nodes.find((n) => n.id === id);
				if (!n) throw new Error(`Unknown node ${id}`);
				return client({ x: n.x + n.width / 2, y: n.y + 8 });
			},
			edge: (id: string) => {
				const e = current.current.edges.find((e) => e.id === id);
				if (!e?.sourceHandle || !e.targetHandle)
					throw new Error(`Unknown edge ${id}`);
				const a = pin(e.source, e.sourceHandle);
				const b = pin(e.target, e.targetHandle);
				return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
			},
			reset,
			undo,
			redo,
			setViewport: async (x: number, y: number, zoom: number) => {
				await flow.setViewport({ x, y, zoom });
				await pause(50);
			},
			setCommitDelay: (ms: number) => {
				delay.current = ms;
			},
			setActivity: async (count: number) => {
				const runtime = useRunExecutionStore.getState();
				runtime.removeRun(ACTIVITY_RUN_ID);
				if (count > 0) {
					const ids = current.current.nodes
						.filter((node) => !nodeNeedsDom(node))
						.slice(0, count)
						.map((node) => node.id);
					await runtime.addRun(ACTIVITY_RUN_ID, "harness", []);
					runtime.addNodesOnRun(ACTIVITY_RUN_ID, ids);
				}
				await pause(100);
			},
			setReadOnly: (value: boolean) => {
				locked.current = value;
				setReadOnly(value);
			},
			changeScope: () => {
				epoch.current++;
				setScope((s) => s + 1);
			},
			connect,
			disconnect: (id: string) => {
				const e = current.current.edges.find((e) => e.id === id);
				return e ? disconnect(e) : Promise.reject(new Error("Unknown edge"));
			},
		};
		Object.assign(window, { __boardCanvasHarness: api });
		return () => {
			Reflect.deleteProperty(window, "__boardCanvasHarness");
		};
	}, [flow, reset, undo, redo, connect, disconnect]);
	useEffect(() => {
		const onKey = (e: KeyboardEvent) => {
			if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z") {
				e.preventDefault();
				if (e.shiftKey) redo();
				else undo();
			}
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [undo, redo]);
	return (
		<main className="harness" data-revision={version}>
			<div className="harness-toolbar">
				<strong>Integrated egui board preview</strong>
				<span>{count.toLocaleString()} nodes</span>
				<button
					type="button"
					onClick={undo}
					disabled={!history.current.undo.length || pending.current}
				>
					Undo
				</button>
				<button
					type="button"
					onClick={redo}
					disabled={!history.current.redo.length || pending.current}
				>
					Redo
				</button>
				<button type="button" onClick={() => void reset()}>
					Reset
				</button>
				<label>
					<input
						type="checkbox"
						checked={readOnly}
						onChange={(e) => setReadOnly(e.target.checked)}
					/>{" "}
					Read only
				</label>
				<FixtureRendererSettings />
			</div>
			<p className="harness-caption">
				Production canvas, gestures, pin validation and command builder.
				Simulated authoritative backend and history; selected node editor is a
				small fixture. Drag pins to create/reconnect, Shift-drag to select,
				Escape to cancel. {pending.current ? "Commit pending…" : message}
			</p>
			<div className="harness-board">
				<EguiFlowCanvas
					nodes={nodes}
					edges={edges}
					nodeTypes={nodeTypes}
					edgeTypes={edgeTypes}
					scopeKey={`harness-${scope}`}
					flowRef={undefined}
					colorMode="dark"
					nodesInteractive={!readOnly}
					onlyRenderVisible={true}
					insideLayer={false}
					onContextMenu={noop}
					onInit={onInit}
					onNodeDoubleClick={noop}
					onEdgeDoubleClick={noop}
					onNodesChange={onNodesChange}
					onEdgesChange={onEdgesChange}
					onNodeDragStop={onNodeDragStop}
					onNodeDrag={noop}
					onNodeDragStart={onNodeDragStart}
					onPaneClick={noop}
					isValidConnection={valid}
					onConnect={legacyConnect}
					onSelectionChange={noop}
					onReconnect={legacyReconnect}
					onReconnectStart={noop}
					onMoveEnd={noop}
					onReconnectEnd={noop}
					onConnectEnd={onConnectEnd}
					onNativeConnection={nativeConnect}
					onNativeCancelDrag={cancelDrag}
					onNativeDisconnect={disconnect}
					onScreenshot={capture}
					miniMapNodeColor={mapColor}
					overlay={<FlowHelperLinesLayer lines={helper.lines} />}
				/>
			</div>
		</main>
	);
}

const root = document.getElementById("root");
if (!root) throw new Error("Missing app root");
initializeFixtureRenderer();
createRoot(root).render(
	<BoardRuntimeProvider>
		<Harness />
	</BoardRuntimeProvider>,
);
