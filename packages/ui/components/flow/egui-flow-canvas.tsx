"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Background,
	BackgroundVariant,
	ControlButton,
	Controls,
	MiniMap,
	type NodeMouseHandler,
	ReactFlow,
	ReactFlowProvider,
} from "@xyflow/react";
import { ShareIcon } from "lucide-react";
import {
	Component,
	type ReactNode,
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { flushSync } from "react-dom";
import { useBoardRenderer } from "../../hooks/use-board-renderer";
import { useWebGlSupport } from "../../hooks/use-webgl-support";
import { domRendererMinZoom } from "../../lib/board-canvas/dom-visibility";
import {
	type MeasuredNodeGeometry,
	collectDomNodeIds,
	isNativeCanvasNode,
	sceneFromFlow,
} from "../../lib/board-canvas/scene";
import { collectNodeStatus } from "../../lib/board-canvas/status";
import { readCanvasTheme } from "../../lib/board-canvas/theme";
import { type BoardTrace, NO_TRACE } from "../../lib/board-canvas/trace";
import { isWebkitLite } from "../../lib/platform";
import { createSignalStore } from "../../lib/realtime/presence-signals-store";
import { useBoardQualityStore } from "../../state/board-quality-state";
import { useLogAggregation } from "../../state/log-aggregation-state";
import { useRunExecutionStore } from "../../state/run-execution-state";
import { BoardRendererStatus } from "./board-renderer-status";
import {
	LegacyBoardRuntimeBridge,
	useReactFlow,
	useStore,
	useStoreApi,
	zoomAboutCenter,
} from "./board-runtime";
import { EguiCanvasLayer } from "./egui-canvas-layer";
import {
	createDomFlowGeometryCache,
	createNativeFlowProjectionCache,
	prepareNativeSceneNodes,
} from "./egui-flow-proxy";
import { LargeGraphNavigation } from "./large-graph-navigation";
import {
	StandaloneBoardBackground,
	StandaloneBoardControls,
	StandaloneBoardMiniMap,
} from "./standalone-board-chrome";
import {
	NATIVE_EDGE_PLANE_Z,
	NATIVE_NODE_PLANE_Z,
	StandaloneBoardHost,
} from "./standalone-board-host";
import { WireTraceContext } from "./wire-trace";

import type { FlowCanvasProps } from "./flow-board";

const MAX_ZOOM = 3;
const NATIVE_MIN_ZOOM = 0.02;

function isTextEditingTarget(target: EventTarget | null) {
	const element =
		target instanceof Element
			? target
			: target instanceof globalThis.Node
				? target.parentElement
				: null;
	return !!element?.closest(
		"input, textarea, [contenteditable]:not([contenteditable='false'])",
	);
}

function NativeViewportPlane({
	zIndex,
	children,
}: { zIndex: number; children: ReactNode }) {
	return (
		<div
			data-native-canvas-plane={zIndex}
			style={{
				position: "absolute",
				inset: 0,
				zIndex,
				pointerEvents: "none",
				transformOrigin: "0 0",
				// Keep transparent canvases on stable compositor layers as DOM editors
				// move or unmount. These viewport-sized planes never scale their pixels.
				transform: "translateZ(0)",
				contain: "paint",
			}}
		>
			{children}
		</div>
	);
}

class CanvasBoundary extends Component<
	{ children: ReactNode; onError: (error: Error) => void },
	{ failed: boolean }
> {
	state = { failed: false };
	static getDerivedStateFromError() {
		return { failed: true };
	}
	componentDidCatch(error: Error) {
		this.props.onError(error);
	}
	render() {
		return this.state.failed ? null : this.props.children;
	}
}

function LegacyCanvas({
	exporting: _exporting,
	...props
}: React.ComponentProps<typeof ReactFlow> & { exporting?: boolean }) {
	const [initialView] = useState(() => ({
		defaultViewport: props.defaultViewport,
		fitView: props.fitView,
	}));
	return (
		<ReactFlowProvider>
			<LegacyBoardRuntimeBridge>
				<ReactFlow {...props} {...initialView} />
			</LegacyBoardRuntimeBridge>
		</ReactFlowProvider>
	);
}

export function EguiFlowCanvas(props: FlowCanvasProps) {
	const { t } = useTranslation("flow");
	const store = useStoreApi();
	const flow = useReactFlow();
	const measuredGeometry = useStore((state) => state.domMeasuredNodeLookup);
	const host = useRef<HTMLDivElement>(null);
	const { renderer: preferredRenderer, ready: preferenceReady } =
		useBoardRenderer();
	const [startupAttempt, setStartupAttempt] = useState(0);
	const preferenceSession = useMemo(
		() => ({ renderer: preferredRenderer, attempt: startupAttempt }),
		[preferredRenderer, startupAttempt],
	);
	const [fallback, setFallback] = useState<{
		preference: typeof preferenceSession;
		message: string;
	}>();
	// Without WebGL2 egui can only fail, after loading its WASM and parsing the whole scene.
	const webGl2 = useWebGlSupport("webgl2");
	const unsupported =
		preferredRenderer === "egui" && webGl2 === false
			? t(
					"boardRendererNoWebGl2",
					"This browser has no WebGL2, which the WASM renderer needs.",
				)
			: undefined;
	const failure =
		unsupported ??
		(fallback?.preference === preferenceSession ? fallback.message : undefined);
	const renderer = failure ? "react-flow" : preferredRenderer;
	const paneWidth = useStore((state) => state.width);
	const paneHeight = useStore((state) => state.height);
	// React Flow mounts every node in view as a rich editor, so its floor follows
	// the board's density. Before React Flow measures its pane, the window bounds it.
	const minZoom = useMemo(
		() =>
			renderer === "egui"
				? NATIVE_MIN_ZOOM
				: domRendererMinZoom(
						props.nodes ?? [],
						paneWidth || globalThis.innerWidth || 0,
						paneHeight || globalThis.innerHeight || 0,
					),
		[renderer, props.nodes, paneWidth, paneHeight],
	);
	const [flowMounted, setFlowMounted] = useState(false);
	const [readyScopes, setReadyScopes] = useState<{
		preference?: typeof preferenceSession;
		nodes?: string;
		edges?: string;
	}>({});
	const [hoveredNodeId, setHoveredNodeId] = useState<string>();
	const hoveredNode = useRef<string | undefined>(undefined);
	const hoverIntent = useRef<{
		id?: string;
		timer?: ReturnType<typeof setTimeout>;
	}>({});
	const pointerHeld = useRef(false);
	const viewportMoving = useRef(false);
	const [projection] = useState(createNativeFlowProjectionCache);
	const [domGeometry] = useState(createDomFlowGeometryCache);
	const [wireTrace] = useState(() => createSignalStore<BoardTrace>(NO_TRACE));
	const [retainedDomIds, setRetainedDomIds] = useState<Set<string>>(
		() => new Set(),
	);
	const [exporting, setExporting] = useState(false);
	const screenshotBusy = useRef(false);
	const currentScope = useRef(props.scopeKey);
	currentScope.current = props.scopeKey;
	const [theme, setTheme] = useState<ReturnType<typeof readCanvasTheme>>();
	const runs = useRunExecutionStore((state) =>
		renderer === "egui" ? state.runs : undefined,
	);
	const currentMetadata = useLogAggregation((state) =>
		renderer === "egui" ? state.currentMetadata : undefined,
	);
	const currentSummary = useLogAggregation((state) =>
		renderer === "egui" ? state.currentSummary : undefined,
	);
	const currentSummaryRunId = useLogAggregation((state) =>
		renderer === "egui" ? state.currentSummaryRunId : undefined,
	);
	const heatmapEnabled = useLogAggregation((state) =>
		renderer === "egui" ? state.heatmapEnabled : undefined,
	);
	const heatmap = useLogAggregation((state) =>
		renderer === "egui" ? state.heatmap : undefined,
	);
	const qualityInline = useBoardQualityStore((state) =>
		renderer === "egui" ? state.inline : undefined,
	);
	const qualityMarks = useBoardQualityStore((state) =>
		renderer === "egui" ? state.marks : undefined,
	);
	const nodeStatus = useMemo(
		() =>
			renderer === "egui"
				? collectNodeStatus(props.nodes ?? [], {
						runs,
						currentMetadata,
						currentSummary,
						currentSummaryRunId,
						heatmapEnabled,
						heatmap,
						qualityInline,
						qualityMarks,
					})
				: undefined,
		[
			renderer,
			props.nodes,
			runs,
			currentMetadata,
			currentSummary,
			currentSummaryRunId,
			heatmapEnabled,
			heatmap,
			qualityInline,
			qualityMarks,
		],
	);
	const previousDomIds = useRef<Set<string>>(new Set());
	const domNodeIds = useMemo(() => {
		if (renderer !== "egui") return new Set<string>();
		const ids = collectDomNodeIds(props.nodes ?? [], {
			forceDomText: theme?.nativeFontCompatible === false,
		});
		for (const id of retainedDomIds) ids.add(id);
		if (
			ids.size === previousDomIds.current.size &&
			[...ids].every((id) => previousDomIds.current.has(id))
		)
			return previousDomIds.current;
		previousDomIds.current = ids;
		return ids;
	}, [renderer, props.nodes, retainedDomIds, theme?.nativeFontCompatible]);
	const nativePalette = useMemo(
		() =>
			theme
				? {
						...theme.palette,
						background: [
							theme.palette.background[0],
							theme.palette.background[1],
							theme.palette.background[2],
							0,
						] as const,
					}
				: undefined,
		[theme],
	);
	const pruneHover = useRef(() => {});
	const applyHover = useCallback((id?: string) => {
		if (hoveredNode.current === id) return;
		hoveredNode.current = id;
		setHoveredNodeId(id);
		if (id)
			setRetainedDomIds((previous) =>
				previous.has(id) ? previous : new Set([...previous, id]),
			);
		pruneHover.current();
	}, []);
	const onHoverNode = useCallback(
		(id?: string) => {
			clearTimeout(hoverIntent.current.timer);
			hoverIntent.current.id = id;
			if (pointerHeld.current || viewportMoving.current) return;
			if (!id || hoveredNode.current === id) {
				applyHover(id);
				return;
			}
			// A pointer crossing a node on its way to a pan should not mount its editor.
			hoverIntent.current.timer = setTimeout(() => {
				if (!pointerHeld.current && !viewportMoving.current) applyHover(id);
			}, 80);
		},
		[applyHover],
	);
	const onViewportMoveStart = useCallback(() => {
		// Wheel and animated viewport changes can move nodes beneath a stationary pointer.
		viewportMoving.current = true;
		clearTimeout(hoverIntent.current.timer);
	}, []);
	const onViewportMoveEnd = useCallback<
		NonNullable<FlowCanvasProps["onMoveEnd"]>
	>(
		(event, viewport) => {
			viewportMoving.current = false;
			onHoverNode(hoverIntent.current.id);
			pruneHover.current();
			props.onMoveEnd?.(event, viewport);
		},
		[onHoverNode, props.onMoveEnd],
	);
	const onNodeMouseEnter = useCallback<NodeMouseHandler>(
		(_event, node) => {
			if (renderer === "egui" && isNativeCanvasNode(node)) onHoverNode(node.id);
		},
		[renderer, onHoverNode],
	);
	const onNodeMouseLeave = useCallback<NodeMouseHandler>(
		(_event, node) => {
			if (hoveredNode.current === node.id || hoverIntent.current.id === node.id)
				onHoverNode();
		},
		[onHoverNode],
	);
	useEffect(() => {
		if (renderer !== "egui") return;
		const hold = (event: PointerEvent) => {
			if (
				!(event.target instanceof globalThis.Node) ||
				!host.current?.contains(event.target)
			)
				return;
			pointerHeld.current = true;
			clearTimeout(hoverIntent.current.timer);
		};
		const release = () => {
			pointerHeld.current = false;
			pruneHover.current();
		};
		document.addEventListener("pointerdown", hold, true);
		document.addEventListener("pointerup", release, true);
		document.addEventListener("pointercancel", release, true);
		window.addEventListener("blur", release);
		return () => {
			clearTimeout(hoverIntent.current.timer);
			pointerHeld.current = false;
			viewportMoving.current = false;
			document.removeEventListener("pointerdown", hold, true);
			document.removeEventListener("pointerup", release, true);
			document.removeEventListener("pointercancel", release, true);
			window.removeEventListener("blur", release);
		};
	}, [renderer]);
	useEffect(() => {
		const element = host.current;
		if (!element) return;
		// WebKit grows a press on board chrome the canvas leaves to the DOM (toolbar gaps,
		// the loading overlay) into a page-wide selection, even under `user-select: none`.
		const blockSelection = (event: Event) => {
			if (!isTextEditingTarget(event.target)) event.preventDefault();
		};
		element.addEventListener("selectstart", blockSelection);
		return () => element.removeEventListener("selectstart", blockSelection);
	}, []);
	// biome-ignore lint/correctness/useExhaustiveDependencies: Hover editors belong to the active board and renderer session.
	useEffect(() => {
		clearTimeout(hoverIntent.current.timer);
		hoverIntent.current = {};
		viewportMoving.current = false;
		hoveredNode.current = undefined;
		setHoveredNodeId(undefined);
		setRetainedDomIds(new Set());
	}, [props.scopeKey, renderer]);
	useEffect(() => {
		if (renderer !== "egui" || !retainedDomIds.size) return;
		let timer: ReturnType<typeof setTimeout>;
		const prune = () => {
			if (pointerHeld.current || viewportMoving.current) return;
			// An editor owns its dialogs and menus even after the pointer leaves its node.
			if (
				document.querySelector(
					'[role="dialog"][data-state="open"], [role="menu"][data-state="open"], [data-radix-popper-content-wrapper]',
				)
			)
				return;
			const focused =
				document.activeElement?.closest<HTMLElement>(".react-flow__node")
					?.dataset.id;
			setRetainedDomIds((previous) => {
				const next = new Set(
					[...previous].filter(
						(id) => id === hoveredNode.current || id === focused,
					),
				);
				return next.size === previous.size ? previous : next;
			});
		};
		const schedule = () => {
			clearTimeout(timer);
			timer = setTimeout(prune, 180);
		};
		pruneHover.current = schedule;
		const observer = new MutationObserver(schedule);
		observer.observe(document.body, {
			subtree: true,
			childList: true,
			attributes: true,
			attributeFilter: ["data-state"],
		});
		document.addEventListener("focusin", schedule);
		document.addEventListener("focusout", schedule);
		document.addEventListener("pointerdown", schedule);
		return () => {
			clearTimeout(timer);
			observer.disconnect();
			document.removeEventListener("focusin", schedule);
			document.removeEventListener("focusout", schedule);
			document.removeEventListener("pointerdown", schedule);
			pruneHover.current = () => {};
		};
	}, [renderer, retainedDomIds.size]);
	const fail = useCallback(
		(error: Error) => {
			setFallback({
				preference: preferenceSession,
				message: error.message || "The native renderer could not start.",
			});
			setReadyScopes({});
		},
		[preferenceSession],
	);
	const retry = useCallback(() => {
		setFallback(undefined);
		setReadyScopes({});
		setStartupAttempt((attempt) => attempt + 1);
	}, []);
	// biome-ignore lint/correctness/useExhaustiveDependencies: Mounting the canvas or changing its color mode changes the scoped theme source.
	useEffect(() => {
		if (renderer !== "egui") return;
		let disposed = false;
		const flowHost = host.current?.querySelector<HTMLElement>(".react-flow");
		const refresh = () => {
			if (disposed || !host.current) return;
			try {
				const next = readCanvasTheme(flowHost ?? host.current);
				setTheme((previous) =>
					JSON.stringify(previous) === JSON.stringify(next) ? previous : next,
				);
			} catch (error) {
				fail(error instanceof Error ? error : new Error(String(error)));
			}
		};
		const frame = requestAnimationFrame(refresh);
		void document.fonts?.ready.then(refresh);
		const observer = new MutationObserver(refresh);
		observer.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ["class", "style"],
		});
		if (flowHost)
			observer.observe(flowHost, {
				attributes: true,
				attributeFilter: ["class", "style"],
			});
		if (host.current?.parentElement)
			observer.observe(host.current.parentElement, {
				attributes: true,
				attributeFilter: ["class", "style"],
			});
		return () => {
			disposed = true;
			cancelAnimationFrame(frame);
			observer.disconnect();
		};
	}, [renderer, fail, flowMounted, props.colorMode]);
	const scene = useMemo(() => {
		if (!theme || renderer !== "egui") return undefined;
		const geometry = new Map<string, MeasuredNodeGeometry>();
		const lookup = measuredGeometry ?? store.getState().nodeLookup;
		for (const node of props.nodes ?? []) {
			if (!domNodeIds.has(node.id) || !isNativeCanvasNode(node)) continue;
			const held = lookup.get(node.id);
			if (!held) continue;
			// A catalog update can change pin rows before the host measures the new DOM.
			if (
				(held.data.node ?? held.data.layer) !==
				(node.data.node ?? node.data.layer)
			)
				continue;
			geometry.set(node.id, {
				width: held.measured.width,
				height: held.measured.height,
				handles: [
					...(held.internals.handleBounds?.source ?? [])
						.filter((handle) => handle.id != null)
						.map((handle) => ({
							...handle,
							id: handle.id as string,
							direction: "output" as const,
						})),
					...(held.internals.handleBounds?.target ?? [])
						.filter((handle) => handle.id != null)
						.map((handle) => ({
							...handle,
							id: handle.id as string,
							direction: "input" as const,
						})),
				],
			});
		}
		const result = sceneFromFlow(
			prepareNativeSceneNodes(props.nodes ?? [], domNodeIds),
			props.edges ?? [],
			theme.palette,
			{
				geometry,
				pinColors: theme.pinColors,
				referenceColor: theme.referenceColor,
				headerColors: theme.headerColors,
				domNodeIds,
				webkitLite: isWebkitLite(),
				rootFontSize: theme.rootFontSize,
				nodeRadius: theme.nodeRadius,
				rerouteRadius: theme.rerouteRadius,
			},
		);
		return result;
	}, [
		props.nodes,
		props.edges,
		renderer,
		store,
		theme,
		domNodeIds,
		measuredGeometry,
	]);
	const active =
		renderer === "egui" &&
		readyScopes.preference === preferenceSession &&
		readyScopes.nodes === props.scopeKey &&
		readyScopes.edges === props.scopeKey &&
		!!scene &&
		!exporting;
	const nativePlanned = renderer === "egui" && !!scene && !exporting;
	const CanvasHost = renderer === "egui" ? StandaloneBoardHost : LegacyCanvas;
	const nativePending = renderer === "egui" && !active && !exporting;
	useLayoutEffect(() => {
		if (preferenceReady && (renderer === "react-flow" || scene))
			setFlowMounted(true);
	}, [preferenceReady, renderer, scene]);
	const nativeBoardProps = useMemo(
		() => ({
			...props,
			onMoveEnd: onViewportMoveEnd,
			nodesInteractive: props.nodesInteractive && !nativePending,
		}),
		[props, nativePending, onViewportMoveEnd],
	);
	const edges = useMemo(
		() =>
			nativePlanned && scene
				? projection.edges(props.edges ?? [], scene)
				: props.edges,
		[nativePlanned, scene, props.edges, projection],
	);
	const nodes = useMemo(
		() =>
			nativePlanned && scene
				? projection.nodes(props.nodes ?? [], scene, hoveredNodeId)
				: renderer === "react-flow"
					? domGeometry(props.nodes ?? [])
					: props.nodes,
		[
			nativePlanned,
			scene,
			props.nodes,
			hoveredNodeId,
			projection,
			renderer,
			domGeometry,
		],
	);
	const onNodesReady = useCallback(
		() =>
			setReadyScopes((previous) => ({
				...(previous.preference === preferenceSession ? previous : {}),
				preference: preferenceSession,
				nodes: props.scopeKey,
			})),
		[props.scopeKey, preferenceSession],
	);
	const onEdgesReady = useCallback(
		() =>
			setReadyScopes((previous) => ({
				...(previous.preference === preferenceSession ? previous : {}),
				preference: preferenceSession,
				edges: props.scopeKey,
			})),
		[props.scopeKey, preferenceSession],
	);
	const handoverViewport = () => {
		const viewport = flow.getViewport();
		const { width, height } = store.getState();
		return zoomAboutCenter(
			viewport,
			Math.max(minZoom, Math.min(MAX_ZOOM, viewport.zoom)),
			width,
			height,
		);
	};
	useEffect(() => {
		if (renderer === "react-flow" && flow.getViewport().zoom < minZoom)
			void flow.zoomTo(minZoom);
	}, [renderer, minZoom, flow]);
	const onScreenshot = useCallback(async () => {
		if (screenshotBusy.current) return;
		screenshotBusy.current = true;
		const scope = props.scopeKey;
		try {
			if (active) {
				// Mount export content in the existing viewport before the exporter clones it.
				flushSync(() => {
					setExporting(true);
					setReadyScopes({});
				});
				await document.fonts?.ready;
				await new Promise<void>((resolve) => {
					requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
				});
				if (!host.current || currentScope.current !== scope) return;
			}
			await props.onScreenshot();
		} finally {
			if (active && host.current) setExporting(false);
			screenshotBusy.current = false;
		}
	}, [active, props.onScreenshot, props.scopeKey]);
	return (
		<div
			ref={host}
			className="relative h-full w-full"
			data-board-renderer={
				!preferenceReady || !flowMounted || nativePending
					? "loading"
					: active
						? "egui"
						: "react-flow"
			}
			data-board-exporting={exporting || undefined}
			aria-busy={
				Boolean(props.loadingStatus) || !preferenceReady || nativePending
			}
		>
			{flowMounted && (
				<WireTraceContext.Provider
					value={renderer === "egui" ? wireTrace : undefined}
				>
					<CanvasHost
						exporting={exporting}
						defaultViewport={handoverViewport()}
						suppressHydrationWarning
						deleteKeyCode={null}
						onContextMenu={props.onContextMenu}
						nodesDraggable={props.nodesInteractive && !nativePending}
						nodesConnectable={props.nodesInteractive && !nativePending}
						onlyRenderVisibleElements={
							exporting ? false : props.onlyRenderVisible
						}
						onInit={props.onInit}
						onNodeMouseEnter={onNodeMouseEnter}
						onNodeMouseLeave={onNodeMouseLeave}
						snapToGrid={false}
						ref={props.flowRef}
						colorMode={props.colorMode}
						nodes={nodes}
						nodeTypes={props.nodeTypes}
						edges={edges}
						edgeTypes={props.edgeTypes}
						maxZoom={MAX_ZOOM}
						minZoom={minZoom}
						onNodeDoubleClick={props.onNodeDoubleClick}
						onEdgeDoubleClick={props.onEdgeDoubleClick}
						onNodesChange={props.onNodesChange}
						onEdgesChange={props.onEdgesChange}
						onNodeDragStop={props.onNodeDragStop}
						onNodeDrag={props.onNodeDrag}
						onNodeDragStart={props.onNodeDragStart}
						onSelectionDragStart={(event, nodes) => {
							if (nodes[0]) props.onNodeDragStart?.(event, nodes[0], nodes);
						}}
						onSelectionDrag={(event, nodes) => {
							if (nodes[0]) props.onNodeDrag?.(event, nodes[0], nodes);
						}}
						onSelectionDragStop={(event, nodes) => {
							if (nodes[0]) props.onNodeDragStop?.(event, nodes[0], nodes);
						}}
						onPaneClick={props.onPaneClick}
						isValidConnection={props.isValidConnection}
						onConnect={props.onConnect}
						onSelectionChange={props.onSelectionChange}
						onReconnect={props.onReconnect}
						onReconnectStart={props.onReconnectStart}
						onMoveStart={renderer === "egui" ? onViewportMoveStart : undefined}
						onMoveEnd={
							renderer === "egui" ? onViewportMoveEnd : props.onMoveEnd
						}
						onReconnectEnd={props.onReconnectEnd}
						onConnectEnd={props.onConnectEnd}
						fitView={renderer === "egui" || !flow.viewportInitialized}
						proOptions={{ hideAttribution: true }}
					>
						{renderer === "egui" && scene && nativePalette && !exporting && (
							<CanvasBoundary key={props.scopeKey} onError={fail}>
								<NativeViewportPlane zIndex={NATIVE_EDGE_PLANE_Z}>
									<EguiCanvasLayer
										scene={scene}
										palette={nativePalette}
										boardProps={nativeBoardProps}
										onError={fail}
										onReady={onEdgesReady}
										lod
										paintLayer="edges"
										status={nodeStatus}
										trace={wireTrace}
										active={active}
										interactive={false}
									/>
								</NativeViewportPlane>
								<NativeViewportPlane zIndex={NATIVE_NODE_PLANE_Z}>
									<EguiCanvasLayer
										scene={scene}
										palette={nativePalette}
										boardProps={nativeBoardProps}
										onError={fail}
										onReady={onNodesReady}
										lod
										paintLayer="nodes"
										status={nodeStatus}
										trace={wireTrace}
										runs={runs}
										onHoverNode={onHoverNode}
									/>
								</NativeViewportPlane>
							</CanvasBoundary>
						)}
						{props.overlay}
						{!exporting && !nativePending && !props.loadingStatus && (
							<LargeGraphNavigation
								// Sibling keys must differ from the canvas boundary's, or React drops one on export.
								key={`navigation:${props.scopeKey}`}
								nodes={props.nodes ?? []}
								native={renderer === "egui"}
							/>
						)}
						{renderer === "egui" ? (
							<>
								<StandaloneBoardControls
									onScreenshot={onScreenshot}
									exporting={exporting}
								/>
								<StandaloneBoardMiniMap
									nodes={nodes ?? []}
									nodeColor={props.miniMapNodeColor}
									themeKey={theme}
								/>
								<StandaloneBoardBackground insideLayer={props.insideLayer} />
							</>
						) : (
							<>
								<Controls>
									<ControlButton
										onClick={onScreenshot}
										disabled={exporting}
										aria-busy={exporting}
										aria-label={t("captureScreenshot", "Capture screenshot")}
									>
										<ShareIcon className="size-4" />
									</ControlButton>
								</Controls>
								<MiniMap
									pannable
									zoomable
									bgColor={
										isWebkitLite()
											? "var(--background)"
											: "color-mix(in oklch, var(--background) 80%, transparent)"
									}
									maskColor="rgb(127 127 127 / 0.15)"
									nodeColor={props.miniMapNodeColor}
								/>
								<Background
									variant={
										props.insideLayer
											? BackgroundVariant.Lines
											: BackgroundVariant.Dots
									}
									color={
										props.insideLayer
											? "color-mix(in oklch, var(--foreground) 5%, transparent)"
											: "color-mix(in oklch, var(--foreground) 20%, transparent)"
									}
									bgColor="color-mix(in oklch, var(--background) 80%, transparent)"
									gap={12}
									size={1}
								/>
							</>
						)}
					</CanvasHost>
				</WireTraceContext.Provider>
			)}
			{(props.loadingStatus || !preferenceReady || nativePending) && (
				<output
					className="absolute inset-0 z-40 flex items-center justify-center bg-background/60"
					data-board-canvas-loading
					aria-live="polite"
				>
					<span className="rounded-md border border-border bg-card px-3 py-2 text-sm">
						{props.loadingStatus?.content ??
							(!preferenceReady
								? t("boardLoadingPreferences", "Reading canvas preferences…")
								: !scene
									? t(
											"boardPreparingCanvas",
											"Preparing {{nodeCount}} nodes…",
											{
												nodeCount: (props.nodes?.length ?? 0).toLocaleString(),
											},
										)
									: readyScopes.nodes !== props.scopeKey
										? t("boardDrawingNodes", "Drawing {{nodeCount}} nodes…", {
												nodeCount: scene.nodes.length.toLocaleString(),
											})
										: t(
												"boardDrawingConnections",
												"Drawing {{edgeCount}} connections…",
												{
													edgeCount: scene.edges.length.toLocaleString(),
												},
											))}
					</span>
				</output>
			)}
			<div className="absolute right-3 top-3 z-50">
				<BoardRendererStatus
					state={
						exporting
							? "exporting"
							: !preferenceReady || !flowMounted || nativePending
								? "loading"
								: active
									? "active"
									: failure
										? "fallback"
										: "react-flow"
					}
					error={failure}
					onRetry={failure && !unsupported ? retry : undefined}
				/>
			</div>
		</div>
	);
}
