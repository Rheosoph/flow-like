"use client";
import {
	BoardRuntimeProvider,
	useReactFlow,
} from "../../packages/ui/components/flow/board-runtime";

import { I18nProvider } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
	type Connection,
	type Edge,
	type Node,
	applyEdgeChanges,
	applyNodeChanges,
} from "@xyflow/react";
import { ThemeProvider } from "next-themes";
import { useCallback, useEffect, useRef, useState } from "react";
import { Toaster } from "sonner";
import { CommentNode } from "../../packages/ui/components/flow/comment-node";
import { EguiFlowCanvas } from "../../packages/ui/components/flow/egui-flow-canvas";
import type { FlowCanvasProps } from "../../packages/ui/components/flow/flow-board";
import { FlowCursorsLayer } from "../../packages/ui/components/flow/flow-cursors";
import { FlowDataEdge } from "../../packages/ui/components/flow/flow-data-edge";
import { FlowDragGhostsLayer } from "../../packages/ui/components/flow/flow-drag-ghosts";
import { FlowExecutionEdge } from "../../packages/ui/components/flow/flow-execution-edge";
import { FlowNode } from "../../packages/ui/components/flow/flow-node";
import { FlowPingsLayer } from "../../packages/ui/components/flow/flow-pings";
import { FlowVeilEdge } from "../../packages/ui/components/flow/flow-veil-edge";
import { LayerInnerNode } from "../../packages/ui/components/flow/layer-inner-node";
import { LayerNode } from "../../packages/ui/components/flow/layer-node";
import { MediaNode } from "../../packages/ui/components/flow/media-node";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import type { CursorStore } from "../../packages/ui/hooks/use-realtime-collaboration";
import {
	buildConnectionCommands,
	buildDisconnectionCommands,
} from "../../packages/ui/lib/board-canvas/connection-commands";
import { updateNodeCommand } from "../../packages/ui/lib/command/generic-command";
import {
	isValidConnection,
	parseBoard,
} from "../../packages/ui/lib/flow-board-utils";
import {
	type PeerDrag,
	type PeerPing,
	createSignalStore,
} from "../../packages/ui/lib/realtime/presence-signals-store";
import type {
	IBoard,
	ILayer,
	INode,
} from "../../packages/ui/lib/schema/flow/board";
import type { IGenericCommand } from "../../packages/ui/lib/schema/flow/board/commands/generic-command";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import { useBoardQualityStore } from "../../packages/ui/state/board-quality-state";
import { useLogAggregation } from "../../packages/ui/state/log-aggregation-state";
import { useRunExecutionStore } from "../../packages/ui/state/run-execution-state";
import {
	FixtureRendererSettings,
	useFixtureRendererReady,
} from "../board-canvas-browser/renderer-settings";
import { applyParityCommands, makeParityBoard } from "./model";
const nodeTypes = {
	node: FlowNode,
	callFunctionNode: FlowNode,
	commentNode: CommentNode,
	mediaNode: MediaNode,
	layerNode: LayerNode,
	layerInnerNode: LayerInnerNode,
};
const edgeTypes = {
	data: FlowDataEdge,
	execution: FlowExecutionEdge,
	veil: FlowVeilEdge,
};
const noop = () => {};
const wait = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const peerUsers = new Map([
	[
		"test-peer",
		{
			sub: "test-peer",
			name: "Test peer",
			truncatedName: "Test peer",
			color: "#12b888",
			loading: false,
		},
	],
]);
function Board() {
	const flow = useReactFlow();
	const baseBackend = useBackend();
	const [initialBoard] = useState(() =>
		makeParityBoard(
			Math.min(
				10000,
				Math.max(
					12,
					Number(new URLSearchParams(window.location.search).get("nodes")) ||
						12,
				),
			),
		),
	);
	const board = useRef<IBoard>(initialBoard);
	const [nodes, setNodes] = useState<Node[]>([]);
	const [edges, setEdges] = useState<Edge[]>([]);
	const [layer, setLayer] = useState<string>();
	const [readOnly, setReadOnly] = useState(false);
	const [message, setMessage] = useState(
		"Real production components. The backend and execution events are simulated.",
	);
	const current = useRef({ nodes, edges, layer, readOnly });
	current.current = { nodes, edges, layer, readOnly };
	const cache = useRef<ReturnType<typeof parseBoard>["cache"]>(new Map());
	const commands = useRef<IGenericCommand[]>([]);
	const rejectedCommand = useRef<string | undefined>(undefined);
	const [cursorStore] = useState(() =>
		createSignalStore<ReturnType<CursorStore["getSnapshot"]>>([]),
	);
	const [dragStore] = useState(() => createSignalStore<PeerDrag[]>([]));
	const [pingStore] = useState(() => createSignalStore<PeerPing[]>([]));
	const history = useRef<IBoard[]>([]);
	// Every renderer calls onInit when it mounts, so a renderer switch would reset the camera.
	const cameraPlaced = useRef(false);
	const future = useRef<IBoard[]>([]);
	const publishRef = useRef<() => void>(noop);
	const execute = useCallback(async (batch: IGenericCommand[]) => {
		if (current.current.readOnly)
			throw new Error("Historical board versions are read-only.");
		if (rejectedCommand.current) {
			const message = rejectedCommand.current;
			rejectedCommand.current = undefined;
			throw new Error(message);
		}
		const next = structuredClone(board.current);
		applyParityCommands(next, batch);
		history.current.push(board.current);
		future.current = [];
		board.current = next;
		commands.current.push(...batch);
		publishRef.current();
	}, []);
	const executeOne = useCallback(
		async (command: IGenericCommand) => execute([command]),
		[execute],
	);
	const run = useCallback(async (node: INode) => {
		setMessage(`Started ${node.friendly_name}`);
		await useRunExecutionStore
			.getState()
			.addRun("fixture-run", board.current.id, [node.id]);
		useRunExecutionStore.getState().addNodesOnRun("fixture-run", [node.id]);
	}, []);
	const enter = useCallback((next: ILayer) => setLayer(next.id), []);
	const publish = useCallback(() => {
		const state = current.current;
		const parsed = parseBoard(
			board.current,
			"parity-app",
			() => setMessage("Copy requested"),
			enter,
			run,
			executeOne,
			new Set(state.nodes.filter((n) => n.selected).map((n) => n.id)),
			"default",
			state.nodes,
			state.edges,
			state.layer,
			board,
			state.readOnly ? [0, 0, 1] : undefined,
			(n) => setMessage(`Info: ${n.id}`),
			(ids) => setMessage(`Explain: ${ids.join(", ")}`),
			(id) => setMessage(`Logs filtered for ${id}`),
			{ isOffline: true },
			{
				nodeNames: new Set(
					Object.values(board.current.nodes).map((n) => n.name),
				),
				wasmNodeKeys: new Set(),
			},
		);
		cache.current = parsed.cache;
		setNodes(parsed.nodes);
		setEdges(parsed.edges);
	}, [enter, run, executeOne]);
	publishRef.current = publish;
	// biome-ignore lint/correctness/useExhaustiveDependencies: publish reads them through current, so a change must re-publish.
	useEffect(() => {
		publish();
	}, [publish, layer, readOnly]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: capture the unconfigured backend once; this fixture owns its isolated origin.
	useEffect(() => {
		const backend = {
			...baseBackend,
			boardState: Object.assign(Object.create(baseBackend.boardState), {
				async getBoard() {
					return board.current;
				},
				async executeCommand(
					_app: string,
					_board: string,
					command: IGenericCommand,
				) {
					await executeOne(command);
					return command;
				},
				async executeCommands(
					_app: string,
					_board: string,
					batch: IGenericCommand[],
				) {
					await execute(batch);
					return batch;
				},
				async stopRun() {
					useRunExecutionStore.getState().removeRun("fixture-run");
				},
			}),
			eventState: Object.assign(Object.create(baseBackend.eventState), {
				async cancelExecution(id: string) {
					useRunExecutionStore.getState().removeRun(id);
					setMessage("Run stopped");
				},
			}),
			capabilities: () => ({
				...baseBackend.capabilities(),
				canExecuteLocally: true,
			}),
		};
		useBackendStore.getState().setBackend(backend);
		return () => useBackendStore.setState({ backend: null });
	}, []);
	const connect = useCallback(
		async (connection: Connection, oldEdge?: Edge) => {
			const batch = buildConnectionCommands({
				connection,
				oldEdge,
				boardNodes: board.current.nodes,
				pinCache: cache.current,
				refs: board.current.refs,
			});
			if (!batch) throw new Error("Invalid connection");
			await execute(batch);
		},
		[execute],
	);
	const disconnect = useCallback(
		async (edge: Edge) => {
			const batch = buildDisconnectionCommands({
				edge,
				boardNodes: board.current.nodes,
				pinCache: cache.current,
			});
			if (batch) await execute(batch);
		},
		[execute],
	);
	const onNodesChange = useCallback<
		NonNullable<FlowCanvasProps["onNodesChange"]>
	>((c) => setNodes((n) => applyNodeChanges(c, n)), []);
	const onEdgesChange = useCallback<
		NonNullable<FlowCanvasProps["onEdgesChange"]>
	>((c) => setEdges((e) => applyEdgeChanges(c, e)), []);
	const move = useCallback<NonNullable<FlowCanvasProps["onNodeDragStop"]>>(
		async (_event, node, moved) => {
			const batch = (moved?.length ? moved : [node])
				.filter((n) => board.current.nodes[n.id])
				.map((n) =>
					updateNodeCommand({
						node: {
							...board.current.nodes[n.id],
							coordinates: [n.position.x, n.position.y, 0],
						},
					}),
				);
			if (batch.length) await execute(batch);
		},
		[execute],
	);
	const undo = () => {
		const previous = history.current.pop();
		if (previous) {
			future.current.push(board.current);
			board.current = previous;
			publish();
		}
	};
	const redo = () => {
		const next = future.current.pop();
		if (next) {
			history.current.push(board.current);
			board.current = next;
			publish();
		}
	};
	const runtime = useCallback(async (enabled: boolean) => {
		if (!enabled) {
			useRunExecutionStore.getState().removeRun("fixture-run");
			return;
		}
		await useRunExecutionStore
			.getState()
			.addRun("fixture-run", "parity", ["exec-a"]);
		useRunExecutionStore.getState().addNodesOnRun("fixture-run", ["exec-b"]);
		useRunExecutionStore.getState().removeNodesOnRun("fixture-run", ["exec-a"]);
	}, []);
	const marks = useCallback((enabled: boolean) => {
		useLogAggregation.setState({
			heatmapEnabled: enabled,
			heatmap: enabled
				? {
						nodes: { "integer-a": { visits: 36, errors: 2 } },
						maxVisits: 36,
						runCount: 2,
					}
				: undefined,
			currentMetadata: enabled
				? {
						run_id: "fixture-log",
						nodes: [["integer-b", 3]],
						app_id: "parity-app",
						board_id: "parity",
						end: 100,
						event_id: "exec-a",
						log_level: 3,
						node_id: "integer-b",
						payload: [],
						start: 0,
						version: "0.0.1",
					}
				: undefined,
		});
		const finding = {
			id: "fixture-lint",
			rule: "dead-code",
			severity: "warning",
			target: {
				kind: "node",
				id: "integer-a",
				name: "integer-a",
				layerId: null,
			},
			detail: { kind: "unreachable" },
		} as const;
		useBoardQualityStore.setState({
			marks: enabled
				? {
						parity: {
							"integer-a": {
								severity: "warning",
								counts: { error: 0, warning: 1, info: 0 },
								findings: [finding],
								nested: 0,
								signature: "fixture",
							},
						},
					}
				: {},
		});
	}, []);
	useEffect(() => {
		Object.assign(window, {
			__boardParity: {
				nodeBounds: (id: string) => {
					const node = flow.getNode(id);
					return node ? flow.getNodesBounds([node]) : undefined;
				},
				snapshot: () => ({
					board: structuredClone(board.current),
					bounds: flow.getNodesBounds(flow.getNodes().filter((n) => !n.hidden)),
					visibleNodes: flow.getNodes().filter((n) => !n.hidden).length,
					presentation: {
						nodes: flow.getNodes().length,
						nativeNodes: flow
							.getNodes()
							.filter((n) => n.data.__flowNativeDom === false).length,
						domNodes: flow
							.getNodes()
							.filter((n) => n.data.__flowNativeDom !== false).length,
						edges: flow.getEdges().length,
						nativeEdges: flow
							.getEdges()
							.filter((e) => e.data?.__flowNativeDom === false).length,
					},
					commands: structuredClone(commands.current),
					selected: current.current.nodes
						.filter((n) => n.selected)
						.map((n) => n.id),
					layer: current.current.layer,
					readOnly: current.current.readOnly,
					runs: [...useRunExecutionStore.getState().runs.keys()],
					viewport: flow.getViewport(),
				}),
				setReadOnly,
				setLayer,
				peerSignals: (
					mode: "onscreen" | "offscreen" | "other-layer" | "none",
				) => {
					const enabled = mode !== "none";
					const layerPath = mode === "other-layer" ? "group" : "root";
					const x = mode === "offscreen" ? 4000 : 450;
					cursorStore.set(
						enabled
							? [
									{
										clientId: 17,
										sub: "test-peer",
										layerPath,
										cursor: { x, y: 220 },
									},
								]
							: [],
					);
					dragStore.set(
						enabled
							? [
									{
										clientId: 17,
										sub: "test-peer",
										layerPath,
										nodes: [{ id: "native", x: 600, y: 220 }],
									},
								]
							: [],
					);
					pingStore.set(
						enabled
							? [
									{
										key: `fixture-${mode}`,
										clientId: 17,
										sub: "test-peer",
										layerPath,
										x,
										y: 340,
										seenAt: Date.now(),
										emoji: "👋",
									},
								]
							: [],
					);
				},
				presence: (
					enabled: boolean,
					mode: "selection" | "editor" | "execution" = "selection",
				) =>
					setNodes((ns) =>
						ns.map((n) =>
							n.id !== "native"
								? n
								: {
										...n,
										data: {
											...n.data,
											remoteSelections:
												enabled && mode === "selection"
													? [{ clientId: 17, sub: "test-peer", isActive: true }]
													: [],
											remoteEditors:
												enabled && mode === "editor"
													? [
															{
																clientId: 17,
																sub: "test-peer",
																active: true,
																selected: true,
															},
														]
													: [],
											remoteExecuting: enabled && mode === "execution",
											peerUsers: new Map([
												[
													"test-peer",
													{
														sub: "test-peer",
														name: "Test peer",
														truncatedName: "Test peer",
														color: "#12b888",
														loading: false,
													},
												],
											]),
										},
									},
						),
					),
				runtime,
				marks,
				failNextCommand: (message: string) => {
					rejectedCommand.current = message;
				},
				runtimeNode: async (status: "running" | "done" | "none", ageMs = 0) => {
					const state = useRunExecutionStore.getState();
					if (status === "none") {
						state.removeRun("fixture-native-run");
						return;
					}
					await state.addRun("fixture-native-run", board.current.id, []);
					state.addNodesOnRun("fixture-native-run", ["native"]);
					if (status === "done")
						state.removeNodesOnRun("fixture-native-run", ["native"]);
					useRunExecutionStore.setState((state) => {
						const runs = new Map(state.runs);
						const run = runs.get("fixture-native-run");
						if (run)
							runs.set("fixture-native-run", {
								...run,
								lastNodeUpdateMs: Date.now() - ageMs,
							});
						return { runs };
					});
				},
				logState: (mode: "summary" | "visited" | "none", nodeId = "native") => {
					useLogAggregation.setState({
						currentMetadata:
							mode === "none"
								? undefined
								: {
										run_id: "fixture-summary",
										app_id: "parity-app",
										board_id: board.current.id,
										nodes: mode === "visited" ? [[nodeId, 3]] : [],
										end: 100,
										event_id: "exec-a",
										log_level: 3,
										node_id: nodeId,
										payload: [],
										start: 0,
										version: "0.0.1",
									},
						currentSummaryRunId:
							mode === "summary" ? "fixture-summary" : undefined,
						currentSummary:
							mode === "summary"
								? {
										version: 1,
										fingerprinted: true,
										partial: false,
										total: 6,
										levels: [0, 1, 2, 3, 0],
										nodes: { [nodeId]: [0, 1, 2, 3, 0] },
										groups: [],
										groups_truncated: false,
									}
								: undefined,
					});
				},
				viewport: async (x: number, y: number, zoom: number) => {
					await flow.setViewport({ x, y, zoom });
					await wait(100);
				},
				animateViewport: (
					x: number,
					y: number,
					zoom: number,
					duration: number,
				) => flow.setViewport({ x, y, zoom }, { duration }),
				select: (id: string) =>
					setNodes((ns) => ns.map((n) => ({ ...n, selected: n.id === id }))),
				mutate: (id: string, change: Partial<INode>) => {
					board.current = {
						...board.current,
						nodes: {
							...board.current.nodes,
							[id]: { ...board.current.nodes[id], ...change },
						},
					};
					publish();
				},
				mutateComment: (
					id: string,
					change: Partial<IBoard["comments"][string]>,
				) => {
					board.current = {
						...board.current,
						comments: {
							...board.current.comments,
							[id]: { ...board.current.comments[id], ...change },
						},
					};
					publish();
				},
			},
		});
		return () => {
			Reflect.deleteProperty(window, "__boardParity");
		};
	}, [flow, publish, runtime, marks, cursorStore, dragStore, pingStore]);
	return (
		<main className="parity">
			<div className="parity-bar">
				<FixtureRendererSettings />
				<strong>Production board parity</strong>
				<span>
					{Object.keys(initialBoard.nodes).length.toLocaleString()} nodes
				</span>
				<button type="button" onClick={() => setLayer(undefined)}>
					Root
				</button>
				<button type="button" onClick={undo}>
					Undo
				</button>
				<button type="button" onClick={redo}>
					Redo
				</button>
				<label>
					<input
						type="checkbox"
						checked={readOnly}
						onChange={(e) => setReadOnly(e.target.checked)}
					/>{" "}
					Historical version
				</label>
				<label>
					<input
						type="checkbox"
						onChange={(e) => void runtime(e.target.checked)}
					/>{" "}
					Running
				</label>
				<label>
					<input type="checkbox" onChange={(e) => marks(e.target.checked)} />{" "}
					Logs, heatmap and lint
				</label>
				<span>{message}</span>
			</div>
			<div className="parity-board">
				<EguiFlowCanvas
					nodes={nodes}
					edges={edges}
					nodeTypes={nodeTypes}
					edgeTypes={edgeTypes}
					scopeKey={`parity:${layer ?? "root"}:${readOnly}`}
					flowRef={undefined}
					colorMode="dark"
					nodesInteractive={!readOnly}
					onlyRenderVisible
					insideLayer={!!layer}
					onContextMenu={(e) => {
						e.preventDefault();
						setMessage("Context menu requested");
					}}
					onInit={(instance) => {
						if (cameraPlaced.current) return;
						cameraPlaced.current = true;
						setTimeout(
							() => void instance.setViewport({ x: 20, y: 20, zoom: 1 }),
							250,
						);
					}}
					onNodeDoubleClick={(_e, node) => {
						if (node.type === "layerNode") enter(node.data.layer as ILayer);
					}}
					onEdgeDoubleClick={noop}
					onNodesChange={onNodesChange}
					onEdgesChange={onEdgesChange}
					onNodeDragStop={move}
					onNodeDrag={noop}
					onNodeDragStart={noop}
					onPaneClick={noop}
					isValidConnection={(c) =>
						!current.current.readOnly &&
						isValidConnection(c, cache.current, board.current.refs)
					}
					onConnect={(c) => void connect(c)}
					onSelectionChange={noop}
					onReconnect={(e, c) => void connect(c, e)}
					onReconnectStart={noop}
					onMoveEnd={noop}
					onReconnectEnd={noop}
					onConnectEnd={noop}
					onNativeConnection={connect}
					onNativeDisconnect={disconnect}
					onScreenshot={async () =>
						setMessage("Screenshot DOM projection ready")
					}
					miniMapNodeColor={() => "var(--primary)"}
				/>
				<div data-testid="peer-cursors">
					<FlowCursorsLayer
						store={cursorStore}
						currentLayerPath={layer ?? "root"}
						peerUsers={peerUsers}
					/>
				</div>
				<div data-testid="peer-drags">
					<FlowDragGhostsLayer
						store={dragStore}
						currentLayerPath={layer ?? "root"}
						peerUsers={peerUsers}
					/>
				</div>
				<div data-testid="peer-pings">
					<FlowPingsLayer
						store={pingStore}
						currentLayerPath={layer ?? "root"}
						peerUsers={peerUsers}
					/>
				</div>
			</div>
		</main>
	);
}
export default function Fixture() {
	const rendererReady = useFixtureRendererReady();
	const [query] = useState(
		() => new QueryClient({ defaultOptions: { queries: { retry: false } } }),
	);
	return (
		<QueryClientProvider client={query}>
			<I18nProvider language="en">
				<ThemeProvider attribute="class" defaultTheme="dark" forcedTheme="dark">
					<TooltipProvider>
						<BoardRuntimeProvider>
							{rendererReady && <Board />}
						</BoardRuntimeProvider>
						<Toaster />
					</TooltipProvider>
				</ThemeProvider>
			</I18nProvider>
		</QueryClientProvider>
	);
}
