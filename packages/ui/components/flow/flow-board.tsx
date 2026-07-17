"use client";
import { DragOverlay, useDroppable } from "@dnd-kit/core";
import { createId } from "@paralleldrive/cuid2";
import type { UseQueryResult } from "@tanstack/react-query";
import {
	Background,
	BackgroundVariant,
	type Connection,
	ControlButton,
	Controls,
	type Edge,
	type FinalConnectionState,
	type InternalNode,
	type IsValidConnection,
	MiniMap,
	type Node,
	type OnEdgesChange,
	type OnNodesChange,
	type OnSelectionChangeFunc,
	ReactFlow,
	type ReactFlowInstance,
	addEdge,
	applyEdgeChanges,
	applyNodeChanges,
	getNodesBounds,
	getViewportForBounds,
	reconnectEdge,
	useEdgesState,
	useKeyPress,
	useNodesState,
	useReactFlow,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useMediaQuery } from "@uidotdev/usehooks";
import {
	ArrowBigLeftDashIcon,
	CheckIcon,
	Eye,
	FileCode2Icon,
	FileTextIcon,
	HistoryIcon,
	LayoutTemplateIcon,
	NotebookPenIcon,
	PlayCircleIcon,
	ScrollIcon,
	SearchIcon,
	ShareIcon,
	SparklesIcon,
	SquareChevronUpIcon,
	SquareFunctionIcon,
	VariableIcon,
	WaypointsIcon,
	WifiIcon,
	WifiOffIcon,
	XIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import { useRouter } from "next/navigation";
import {
	type ComponentProps,
	type ReactElement,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type {
	ImperativePanelGroupHandle,
	ImperativePanelHandle,
} from "react-resizable-panels";
import {
	Button,
	Sheet,
	SheetContent,
	SheetHeader,
	SheetTitle,
	useHub,
	useLogAggregation,
	useMobileHeader,
} from "../..";
import { BoardActivityIndicator } from "../../components/flow/board-activity-indicator";
import { CommentNode } from "../../components/flow/comment-node";
import { FlowContextMenu } from "../../components/flow/flow-context-menu";
import { FlowDock } from "../../components/flow/flow-dock";
import { FlowNode } from "../../components/flow/flow-node";
import {
	FlowNodeInfoOverlay,
	type FlowNodeInfoOverlayHandle,
} from "../../components/flow/flow-node/flow-node-info-overlay";
import { FlowPages } from "../../components/flow/flow-pages";
import { FlowScriptPanel } from "../../components/flow/flowscript/flowscript-panel";
import { MediaNode } from "../../components/flow/media-node";
import { Traces } from "../../components/flow/traces";
import { UploadPlaceholderNode } from "../../components/flow/upload-placeholder-node";
import {
	Variable,
	VariablesMenu,
} from "../../components/flow/variables/variables-menu";
import {
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
} from "../../components/ui/resizable";
import { useCommandExecution } from "../../hooks/use-command-execution";
import { useCopilotCommands } from "../../hooks/use-copilot-commands";
import { useExecutionPresence } from "../../hooks/use-execution-presence";
import { useFlowPanels } from "../../hooks/use-flow-panels";
import { useFollowMode } from "../../hooks/use-follow-mode";
import { useInvoke } from "../../hooks/use-invoke";
import { useKeyboardShortcuts } from "../../hooks/use-keyboard-shortcuts";
import { useLayerNavigation } from "../../hooks/use-layer-navigation";
import { useMediaUpload } from "../../hooks/use-media-upload";
import { usePeerUserInfo } from "../../hooks/use-peer-users";
import { useRealtimeChat } from "../../hooks/use-realtime-chat";
import { useRealtimeCollaboration } from "../../hooks/use-realtime-collaboration";
import { useViewportManager } from "../../hooks/use-viewport-manager";
import {
	type IGenericCommand,
	type ILogMetadata,
	IPinType,
	IValueType,
	connectPinsCommand,
	disconnectPinsCommand,
	moveNodeCommand,
	removeCommentCommand,
	removeLayerCommand,
	removeNodeCommand,
	updateNodeCommand,
	upsertCommentCommand,
	upsertVariableCommand,
} from "../../lib";
import { getErrorMessage } from "../../lib/error-message";
import { computeFlowLayout } from "../../lib/flow-auto-layout";
import {
	getFunctionReferenceNodeIdsFromEdge,
	handleConnection,
	handleEdgesChange,
	handleNodesChange,
	handlePlaceNode,
	handlePlacePlaceholder,
	removeFunctionReferenceCommandForEdge,
} from "../../lib/flow-board-helpers";
import {
	handleCopy,
	handlePaste,
	hexToRgba,
	isValidConnection,
	parseBoard,
	shouldIgnoreBoardClipboardEvent,
} from "../../lib/flow-board-utils";
import { toastError, toastSuccess } from "../../lib/messages";
import { getRuntimeConfiguredVariables } from "../../lib/runtime-vars-utils";
import { IAppVisibility } from "../../lib/schema/app/app";
import type { IBit } from "../../lib/schema/bit/bit";
import {
	type IBoard,
	type IComment,
	ICommentType,
	type IVariable,
} from "../../lib/schema/flow/board";
import { type INode, IVariableType } from "../../lib/schema/flow/node";
import type { IPin } from "../../lib/schema/flow/pin";
import type { ILayer } from "../../lib/schema/flow/run";
import { convertJsonToUint8Array } from "../../lib/uint8";
import {
	type AssistantBoardSurface,
	useAssistantSurface,
} from "../../state/assistant-surface";
import { useBackend } from "../../state/backend-state";
import { useFlowBoardParentState } from "../../state/flow-board-parent-state";
import { useRunExecutionStore } from "../../state/run-execution-state";
import {
	type RuntimeVariableValue,
	useRuntimeVariables,
} from "../../state/runtime-variables-context";
import { AutoLayoutDialog, type LayoutStyle } from "./auto-layout-dialog";
import { BoardMeta } from "./board-meta";
import { CallFunctionNode } from "./call-function-node";
import { FlowChat } from "./flow-chat";
import { FlowCopilot } from "./flow-copilot";
import { FlowCursorsLayer } from "./flow-cursors";
import { FlowDataEdge } from "./flow-data-edge";
import { FlowExecutionEdge } from "./flow-execution-edge";
import { useUndoRedo } from "./flow-history";
import { FlowLayerIndicators } from "./flow-layer-indicators";
import { PinEditModal } from "./flow-pin/edit-modal";
import { FlowPresenceBar } from "./flow-presence-bar";
import { FlowRuns } from "./flow-runs";
import { FlowSearch } from "./flow-search";
import {
	type FlowElementOption,
	createEmptyFlowSelectorData,
	flattenPageElements,
	indexBitsByRef,
} from "./flow-selector-data";
import { FlowTemplateSelector } from "./flow-template-selector";
import { FlowVeilEdge } from "./flow-veil-edge";
import { LayerInnerNode } from "./layer-inner-node";
import { LayerNode } from "./layer-node";
import { RuntimeVariablesPrompt } from "./runtime-variables-prompt";
import { WasmSandboxWarningDialog } from "./wasm-sandbox-warning-dialog";

const REMOTE_BOARD_APPLIED_EVENT = "flow:remote-board-applied";

type ReactFlowProps = ComponentProps<typeof ReactFlow>;

interface FlowCanvasProps {
	flowRef: ReactFlowProps["ref"];
	nodes: ReactFlowProps["nodes"];
	edges: ReactFlowProps["edges"];
	nodeTypes: ReactFlowProps["nodeTypes"];
	edgeTypes: ReactFlowProps["edgeTypes"];
	colorMode: ReactFlowProps["colorMode"];
	nodesInteractive: boolean;
	onlyRenderVisible: boolean;
	currentLayer: string | undefined;
	onContextMenu: ReactFlowProps["onContextMenu"];
	onInit: ReactFlowProps["onInit"];
	onNodeDoubleClick: ReactFlowProps["onNodeDoubleClick"];
	onNodesChange: ReactFlowProps["onNodesChange"];
	onEdgesChange: ReactFlowProps["onEdgesChange"];
	onNodeDragStop: ReactFlowProps["onNodeDragStop"];
	onNodeDrag: ReactFlowProps["onNodeDrag"];
	isValidConnection: ReactFlowProps["isValidConnection"];
	onConnect: ReactFlowProps["onConnect"];
	onSelectionChange: ReactFlowProps["onSelectionChange"];
	onReconnect: ReactFlowProps["onReconnect"];
	onReconnectStart: ReactFlowProps["onReconnectStart"];
	onMoveEnd: ReactFlowProps["onMoveEnd"];
	onReconnectEnd: ReactFlowProps["onReconnectEnd"];
	onConnectEnd: ReactFlowProps["onConnectEnd"];
	onScreenshot: () => void;
	miniMapNodeColor: (node: Node) => string;
}

// Memoized so unrelated FlowBoard re-renders (presence, dialogs, copilot toggles,
// menus) skip reconciling the entire React Flow canvas. All props passed in are
// referentially stable (state arrays + useCallback handlers), so the memo holds.
const FlowCanvas = memo(function FlowCanvas({
	flowRef,
	nodes,
	edges,
	nodeTypes,
	edgeTypes,
	colorMode,
	nodesInteractive,
	onlyRenderVisible,
	currentLayer,
	onContextMenu,
	onInit,
	onNodeDoubleClick,
	onNodesChange,
	onEdgesChange,
	onNodeDragStop,
	onNodeDrag,
	isValidConnection,
	onConnect,
	onSelectionChange,
	onReconnect,
	onReconnectStart,
	onMoveEnd,
	onReconnectEnd,
	onConnectEnd,
	onScreenshot,
	miniMapNodeColor,
}: FlowCanvasProps) {
	return (
		<ReactFlow
			suppressHydrationWarning
			deleteKeyCode={null}
			onContextMenu={onContextMenu}
			nodesDraggable={nodesInteractive}
			nodesConnectable={nodesInteractive}
			onlyRenderVisibleElements={onlyRenderVisible}
			onInit={onInit}
			ref={flowRef}
			colorMode={colorMode}
			nodes={nodes}
			nodeTypes={nodeTypes}
			edges={edges}
			edgeTypes={edgeTypes}
			maxZoom={3}
			minZoom={0.1}
			onNodeDoubleClick={onNodeDoubleClick}
			onNodesChange={onNodesChange}
			onEdgesChange={onEdgesChange}
			onNodeDragStop={onNodeDragStop}
			onNodeDrag={onNodeDrag}
			isValidConnection={isValidConnection}
			onConnect={onConnect}
			onSelectionChange={onSelectionChange}
			onReconnect={onReconnect}
			onReconnectStart={onReconnectStart}
			onMoveEnd={onMoveEnd}
			onReconnectEnd={onReconnectEnd}
			onConnectEnd={onConnectEnd}
			fitView
			proOptions={{ hideAttribution: true }}
		>
			<Controls>
				<ControlButton onClick={onScreenshot}>
					<ShareIcon className="size-4" />
				</ControlButton>
			</Controls>
			<MiniMap
				pannable
				zoomable
				bgColor="color-mix(in oklch, var(--background) 80%, transparent)"
				maskColor="color-mix(in oklch, var(--foreground) 10%, transparent)"
				nodeColor={miniMapNodeColor}
			/>
			<Background
				variant={
					currentLayer ? BackgroundVariant.Lines : BackgroundVariant.Dots
				}
				color={
					currentLayer
						? "color-mix(in oklch, var(--foreground) 5%, transparent)"
						: "color-mix(in oklch, var(--foreground) 20%, transparent)"
				}
				bgColor="color-mix(in oklch, var(--background) 80%, transparent)"
				gap={12}
				size={1}
			/>
		</ReactFlow>
	);
});

// Index of each panel inside the outer horizontal ResizablePanelGroup, ordered by
// their `order` prop: variables(0), main container(1), runs(2), flowscript(3), search(4).
const MAIN_PANEL_INDEX = 1;
const RUNS_PANEL_INDEX = 2;
const FLOWSCRIPT_PANEL_INDEX = 3;

export function FlowBoard({
	appId,
	boardId,
	nodeId,
	initialVersion,
	extraDockItems,
	renderOverlay,
	sub,
	externalAssistant = false,
}: Readonly<{
	appId: string;
	boardId: string;
	nodeId?: string;
	initialVersion?: [number, number, number];
	extraDockItems?: Array<{
		title: string;
		icon: React.ReactNode;
		onClick: () => Promise<void> | void;
		separator?: string;
		highlight?: boolean;
		special?: boolean;
	}>;
	renderOverlay?: () => React.ReactNode;
	sub?: string;
	/**
	 * When true the host app provides the assistant (global chat) — FlowPilot launchers route to
	 * requestOpenAssistant() and the embedded FlowCopilot panel/sheet are not mounted.
	 */
	externalAssistant?: boolean;
}>) {
	const {
		pushCommand,
		pushCommands,
		redo,
		undo,
		rollbackUndo,
		rollbackRedo,
		clearHistory,
		stampHistory,
	} = useUndoRedo(appId, boardId);
	const router = useRouter();
	const backend = useBackend();
	const selected = useRef(new Set<string>());
	const hub = useHub();
	const edgeReconnectSuccessful = useRef(true);
	const { isOver, setNodeRef, active } = useDroppable({ id: "flow" });
	const parentRegister = useFlowBoardParentState();
	// Field selectors: the log store also holds currentLogs/isLoading, which
	// churn during runs — subscribing to the whole store re-renders the entire
	// board on every log tick.
	const refetchLogs = useLogAggregation((state) => state.refetchLogs);
	const setCurrentMetadata = useLogAggregation(
		(state) => state.setCurrentMetadata,
	);
	const currentMetadata = useLogAggregation((state) => state.currentMetadata);
	const flowRef = useRef<any>(null);
	const initialVersionKey = initialVersion?.join(".");
	const [version, setVersion] = useState<[number, number, number] | undefined>(
		initialVersion,
	);
	useEffect(() => {
		if (!initialVersionKey) {
			setVersion(undefined);
			return;
		}

		const parts = initialVersionKey.split(".").map(Number);
		if (parts.length !== 3 || parts.some((part) => !Number.isFinite(part))) {
			setVersion(undefined);
			return;
		}

		setVersion([parts[0], parts[1], parts[2]]);
	}, [appId, boardId, initialVersionKey]);
	const [initialized, setInitialized] = useState(false);
	const flowPanelRef = useRef<ImperativePanelHandle>(null);
	const logPanelRef = useRef<ImperativePanelHandle>(null);
	const varPanelRef = useRef<ImperativePanelHandle>(null);

	const runsPanelRef = useRef<ImperativePanelHandle>(null);
	const flowScriptPanelRef = useRef<ImperativePanelHandle>(null);
	const panelGroupRef = useRef<ImperativePanelGroupHandle>(null);
	const nodeInfoOverlayRef = useRef<FlowNodeInfoOverlayHandle>(null);

	const shiftPressed = useKeyPress("Shift");

	const { resolvedTheme } = useTheme();

	const catalog: UseQueryResult<INode[]> = useInvoke(
		backend.boardState.getCatalog,
		backend.boardState,
		[appId],
	);
	const board = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId, boardId, version],
		boardId !== "",
	);
	const boardRef = useRef<IBoard | undefined>(undefined);
	const currentProfile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const selectorDataRef = useRef(createEmptyFlowSelectorData());
	const [selectorDataVersion, setSelectorDataVersion] = useState(0);
	const selectorCacheKeyRef = useRef("");
	const elementOptionsPromiseRef = useRef<
		Promise<FlowElementOption[]> | undefined
	>(undefined);
	const bitOptionsPromiseRef = useRef<Promise<IBit[]> | undefined>(undefined);
	const selectorCacheKey = `${
		currentProfile.data?.hub_profile?.id ??
		backend.profile?.id ??
		backend.profile?.hub ??
		"local"
	}:${appId}`;

	if (selectorCacheKeyRef.current !== selectorCacheKey) {
		selectorCacheKeyRef.current = selectorCacheKey;
		selectorDataRef.current = createEmptyFlowSelectorData();
		elementOptionsPromiseRef.current = undefined;
		bitOptionsPromiseRef.current = undefined;
	}

	const loadElementOptions = useCallback(
		async (force = false) => {
			const cache = selectorDataRef.current;
			if (elementOptionsPromiseRef.current)
				return elementOptionsPromiseRef.current;
			if (!force && cache.elementsLoaded) return cache.elementOptions;

			const cacheKey = selectorCacheKeyRef.current;
			cache.elementsLoading = true;
			cache.elementsError = undefined;

			const promise = (async () => {
				try {
					const [routes, events, pages] = await Promise.all([
						backend.routeState.getRoutes(appId),
						backend.eventState.getEvents(appId),
						backend.pageState.getPages(appId),
					]);
					const eventsMap = new Map(events.map((event) => [event.id, event]));
					const pagesById = new Map(pages.map((page) => [page.pageId, page]));
					const pageTargets = new Map<
						string,
						{ pageName?: string; pagePath?: string; boardId?: string }
					>();

					const queuePage = (
						pageId: string,
						pageName?: string,
						pagePath?: string,
						boardId?: string,
					) => {
						const existing = pageTargets.get(pageId);
						if (existing) {
							existing.pageName ??= pageName;
							existing.pagePath ??= pagePath;
							existing.boardId ??= boardId;
							return;
						}

						pageTargets.set(pageId, { pageName, pagePath, boardId });
					};

					for (const route of routes) {
						const event = eventsMap.get(route.eventId);
						const pageId = event?.default_page_id;
						if (!pageId) continue;

						const pageInfo = pagesById.get(pageId);
						queuePage(
							pageId,
							pageInfo?.name,
							route.path,
							pageInfo?.boardId ?? event.board_id,
						);
					}

					for (const pageInfo of pages) {
						queuePage(
							pageInfo.pageId,
							pageInfo.name,
							undefined,
							pageInfo.boardId,
						);
					}

					const seenIds = new Set<string>();
					const pageElements = await Promise.all(
						Array.from(pageTargets.entries()).map(
							async ([pageId, pageInfo]) => {
								try {
									const page = await backend.pageState.getPage(
										appId,
										pageId,
										pageInfo.boardId,
									);
									return flattenPageElements(page.components ?? []).map(
										(element) => ({
											...element,
											id: `${pageId}/${element.id}`,
											rawId: element.id,
											label: pageInfo.pageName
												? `${pageInfo.pageName} / ${element.label}`
												: element.label,
											pageName: pageInfo.pageName,
											pagePath: pageInfo.pagePath,
										}),
									);
								} catch {
									return [];
								}
							},
						),
					);

					const allElements = pageElements.flat().filter((element) => {
						if (seenIds.has(element.id)) return false;
						seenIds.add(element.id);
						return true;
					});

					if (selectorCacheKeyRef.current === cacheKey) {
						cache.elementOptions = allElements;
						cache.elementsLoaded = true;
						setSelectorDataVersion((current) => current + 1);
					}

					return allElements;
				} catch (error) {
					console.error("Failed to load page elements:", error);
					if (selectorCacheKeyRef.current === cacheKey) {
						cache.elementsError = error;
						setSelectorDataVersion((current) => current + 1);
					}
					return [];
				} finally {
					if (selectorCacheKeyRef.current === cacheKey) {
						cache.elementsLoading = false;
					}
					elementOptionsPromiseRef.current = undefined;
				}
			})();

			elementOptionsPromiseRef.current = promise;
			return promise;
		},
		[appId, backend.eventState, backend.pageState, backend.routeState],
	);

	const loadBitOptions = useCallback(
		async (force = false) => {
			const cache = selectorDataRef.current;
			if (bitOptionsPromiseRef.current) return bitOptionsPromiseRef.current;
			if (!force && cache.bitsLoaded) return cache.bitOptions;

			const cacheKey = selectorCacheKeyRef.current;
			cache.bitsLoading = true;
			cache.bitsError = undefined;

			const promise = (async () => {
				try {
					const bits = await backend.bitState.getProfileBits();
					if (selectorCacheKeyRef.current === cacheKey) {
						cache.bitOptions = bits;
						cache.bitsByRef = indexBitsByRef(bits);
						cache.bitsLoaded = true;
						setSelectorDataVersion((current) => current + 1);
					}
					return bits;
				} catch (error) {
					console.error("Failed to load profile bits:", error);
					if (selectorCacheKeyRef.current === cacheKey) {
						cache.bitsError = error;
						setSelectorDataVersion((current) => current + 1);
					}
					return [];
				} finally {
					if (selectorCacheKeyRef.current === cacheKey) {
						cache.bitsLoading = false;
					}
					bitOptionsPromiseRef.current = undefined;
				}
			})();

			bitOptionsPromiseRef.current = promise;
			return promise;
		},
		[backend.bitState],
	);

	selectorDataRef.current.loadElements = loadElementOptions;
	selectorDataRef.current.loadBits = loadBitOptions;
	const app = useInvoke(backend.appState.getApp, backend.appState, [appId]);
	const { addRun, removeRun, pushUpdate } = useRunExecutionStore();
	const { screenToFlowPosition, getViewport, setViewport, fitView, getNodes } =
		useReactFlow();

	const [nodes, setNodes] = useNodesState<any>([]);
	const [edges, setEdges] = useEdgesState<any>([]);
	const [droppedPin, setDroppedPin] = useState<IPin | undefined>(undefined);
	const [clickPosition, setClickPosition] = useState({ x: 0, y: 0 });
	const deletingNodesRef = useRef<Set<string>>(new Set());
	const mousePositionRef = useRef({ x: 0, y: 0 });
	const [pinCache, setPinCache] = useState<
		Map<string, [IPin, INode | ILayer, boolean]>
	>(new Map());
	const [editBoard, setEditBoard] = useState(false);
	const [currentLayer, setCurrentLayer] = useState<string | undefined>();
	const [layerPath, setLayerPath] = useState<string | undefined>();
	const [templateSelectorOpen, setTemplateSelectorOpen] = useState(false);
	const [runtimeVarsPromptOpen, setRuntimeVarsPromptOpen] = useState(false);
	const [pendingExecution, setPendingExecution] = useState<{
		node: INode;
		payload?: object;
		isRemote: boolean;
	} | null>(null);
	const deleteSelectionInFlightRef = useRef(false);
	const [existingRuntimeVars, setExistingRuntimeVars] = useState<
		Map<string, RuntimeVariableValue>
	>(new Map());
	const runtimeVarsContext = useRuntimeVariables();
	const colorMode = useMemo(
		() => (resolvedTheme === "dark" ? "dark" : "light"),
		[resolvedTheme],
	);

	const { update: updateHeader } = useMobileHeader();

	useEffect(() => {
		const left: ReactElement[] = [];
		const right: ReactElement[] = [];

		if (
			typeof parentRegister.boardParents[boardId] === "string" &&
			!currentLayer
		) {
			left.push(
				<Button
					variant={"default"}
					size={"icon"}
					onClick={async () => {
						const urlWithQuery = parentRegister.boardParents[boardId];
						router.push(urlWithQuery);
					}}
				>
					<ArrowBigLeftDashIcon />
				</Button>,
			);
		}

		right.push(
			...[
				<Button
					variant={"outline"}
					size={"icon"}
					onClick={async () => {
						toggleVars();
					}}
				>
					<VariableIcon />
				</Button>,

				<Button
					variant={"outline"}
					size={"icon"}
					onClick={() => {
						setTemplateSelectorOpen(true);
					}}
				>
					<LayoutTemplateIcon />
				</Button>,
				<Button
					variant={"outline"}
					size={"icon"}
					onClick={async () => {
						setEditBoard(true);
					}}
				>
					<NotebookPenIcon />
				</Button>,
				<Button
					variant={"outline"}
					size={"icon"}
					onClick={async () => {
						toggleRunHistory();
					}}
				>
					<HistoryIcon />
				</Button>,
			],
		);

		// Always expose Logs button; it opens the logs sheet (shows empty state when no run is selected)
		right.push(
			<Button
				variant={"outline"}
				size={"icon"}
				aria-label="Open logs"
				onClick={async () => {
					toggleLogs();
				}}
			>
				<ScrollIcon />
			</Button>,
		);

		// FlowPilot button with fancy styling. When the host provides the global assistant, the
		// floating FlowPilot bubble is the entry point instead, so skip this in-interface button.
		if (!externalAssistant) {
			right.push(
				<Button
					variant={"outline"}
					size={"icon"}
					aria-label="Open FlowPilot"
					onClick={() => openAssistant()}
					className="relative group border-primary/30 hover:border-primary/60 hover:bg-primary/5"
				>
					<div className="absolute inset-0 rounded-md bg-linear-to-br from-primary/20 via-violet-500/10 to-pink-500/10 opacity-0 group-hover:opacity-100 transition-opacity" />
					<SparklesIcon className="w-4 h-4 text-primary relative z-10" />
					{currentMetadata && (
						<span className="absolute -top-1 -right-1 w-2 h-2 bg-amber-500 rounded-full" />
					)}
				</Button>,
			);
		}

		if (currentLayer) {
			left.push(
				<Button
					variant={"default"}
					size={"icon"}
					onClick={async () => {
						popLayer();
					}}
				>
					<SquareChevronUpIcon />
				</Button>,
			);
		}

		updateHeader({
			left,
			right,
		});
	}, [
		currentMetadata,
		currentLayer,
		parentRegister.boardParents,
		boardId,
		updateHeader,
		externalAssistant,
	]);

	const pinToNode = useCallback(
		(pinId: string) => {
			const [_, node] = pinCache.get(pinId) || [];
			return node;
		},
		[nodes, pinCache],
	);

	const { saveViewport } = useViewportManager({
		appId,
		boardId,
		layerPath,
		nodesLength: nodes.length,
	});

	const { focusNode, pushLayer, popLayer } = useLayerNavigation({
		board,
		layerPath,
		setCurrentLayer,
		setLayerPath,
		saveViewport,
		fitView,
		getNodes,
	});

	const {
		executeCommand,
		executeCommands,
		applyFlowScript,
		applyFlowIrCommit,
		awarenessRef: commandAwarenessRef,
	} = useCommandExecution({
		appId,
		boardId,
		board,
		version,
		pushCommand,
		pushCommands,
		stampHistory,
	});

	// Realtime collaboration
	const {
		awareness,
		connectionStatus,
		peerStates,
		cursorStore,
		reconnect,
		broadcastActiveNode,
	} = useRealtimeCollaboration({
		appId,
		boardId,
		board,
		version,
		backend,
		sub,
		hub,
		mousePositionRef,
		layerPath,
		screenToFlowPosition,
		commandAwarenessRef,
		setNodes,
	});

	// Cache peer user info to avoid repeated API calls
	const peerSubs = useMemo(
		() => [
			...new Set(peerStates.map((p) => p.sub).filter((s): s is string => !!s)),
		],
		[peerStates],
	);
	const peerUsers = usePeerUserInfo(
		peerSubs,
		backend.userState.lookupUser.bind(backend.userState),
	);

	// Follow mode
	const { followingSub, toggleFollow, stopFollowing } = useFollowMode({
		awareness,
		sub,
		setViewport,
		getViewport,
	});

	// Build layer name lookup for presence UI
	const layerNames = useMemo(() => {
		const map = new Map<string, string>();
		if (!board.data?.layers) return map;
		for (const [id, layer] of Object.entries(board.data.layers)) {
			if (layer.name) map.set(id, layer.name);
		}
		return map;
	}, [board.data?.layers]);

	// Jump to a peer's location — navigates to their layer and follows their viewport briefly
	// When the same user has multiple sessions, picks the one with the most recent cursor
	const jumpToUser = useCallback(
		(targetSub: string) => {
			if (!awareness) return;
			const states = awareness.getStates() as Map<number, any>;
			let best: { state: any; ts: number } | undefined;
			for (const [clientId, state] of states) {
				if (clientId === awareness.clientID) continue;
				if (state?.sub !== targetSub) continue;
				const ts = (state?.activeNodeTs as number) ?? 0;
				if (!best || ts > best.ts) {
					best = { state, ts };
				}
			}

			if (!best) return;
			const state = best.state;

			const peerLayer = (state?.layerPath as string) ?? "root";
			const myLayer = layerPath ?? "root";

			// Navigate to peer's layer if different
			if (peerLayer !== myLayer) {
				if (peerLayer === "root" || !peerLayer) {
					setLayerPath(undefined);
					setCurrentLayer(undefined);
				} else {
					setLayerPath(peerLayer);
					const segments = peerLayer.split("/");
					setCurrentLayer(segments[segments.length - 1]);
				}
			}

			// Snap to peer's viewport
			const vp = state?.viewport;
			if (vp) {
				setViewport({ x: vp.x, y: vp.y, zoom: vp.zoom }, { duration: 500 });
			} else if (state?.cursor) {
				// Fall back to centering on peer's cursor
				const cursor = state.cursor;
				const currentVp = getViewport();
				const w = typeof window !== "undefined" ? window.innerWidth : 1200;
				const h = typeof window !== "undefined" ? window.innerHeight : 800;
				setViewport(
					{
						x: -cursor.x * currentVp.zoom + w / 2,
						y: -cursor.y * currentVp.zoom + h / 2,
						zoom: currentVp.zoom,
					},
					{ duration: 500 },
				);
			}
		},
		[
			awareness,
			layerPath,
			setViewport,
			getViewport,
			setLayerPath,
			setCurrentLayer,
		],
	);

	// Jump to a specific layer path
	const jumpToLayer = useCallback(
		(targetLayerPath: string) => {
			if (targetLayerPath === "root" || !targetLayerPath) {
				setLayerPath(undefined);
				setCurrentLayer(undefined);
			} else {
				setLayerPath(targetLayerPath);
				const segments = targetLayerPath.split("/");
				setCurrentLayer(segments[segments.length - 1]);
			}
		},
		[setLayerPath, setCurrentLayer],
	);

	// Realtime chat
	const [chatOpen, setChatOpen] = useState(false);
	const handleToggleChat = useCallback(() => setChatOpen((v) => !v), []);
	const {
		messages: chatMessages,
		sendMessage,
		unreadCount,
		setIsOpen: setChatIsOpen,
	} = useRealtimeChat({ awareness, sub });

	// Sync chat open state for unread tracking
	useEffect(() => {
		setChatIsOpen(chatOpen);
	}, [chatOpen, setChatIsOpen]);

	// Execution presence
	const executionRuns = useRunExecutionStore((state) => state.runs);
	const { remoteExecutingNodeIds } = useExecutionPresence({
		awareness,
		sub,
		runs: executionRuns,
		boardId,
	});

	// Media upload for images/videos on the board
	const { handleMediaPaste } = useMediaUpload({
		appId,
		boardId,
		backend,
		executeCommand,
		currentLayer,
		setNodes,
	});

	useEffect(() => {
		if (!logPanelRef.current) return;
		// Avoid auto-expanding logs on mobile to prevent layout jump
		const isMobile =
			typeof window !== "undefined" &&
			window.matchMedia("(max-width: 767px)").matches;
		if (isMobile) return;
		logPanelRef.current.expand();
		const size = logPanelRef.current.getSize();
		if (size < 10) logPanelRef.current.resize(45);
	}, [logPanelRef.current]);

	const initializeFlow = useCallback(
		async (_instance: ReactFlowInstance) => {
			if (initialized) return;
			if (!nodeId || nodeId === "") return;

			focusNode(nodeId);
			setInitialized(true);
		},
		[nodeId, initialized, focusNode],
	);

	// Check if board is empty (no nodes) for showing template selector
	const isBoardEmpty = useMemo(() => {
		if (!board.data) return false;
		const nodeCount = Object.keys(board.data.nodes).length;
		const commentCount = Object.keys(board.data.comments).length;
		const layerCount = Object.keys(board.data.layers).length;
		return nodeCount === 0 && commentCount === 0 && layerCount === 0;
	}, [board.data]);

	// Handler for applying a template to the board
	const handleApplyTemplate = useCallback(
		async (templateAppId: string, templateId: string) => {
			if (typeof version !== "undefined") {
				toastError("Cannot modify old version", <XIcon />);
				return;
			}

			try {
				const templateBoard = await backend.templateState.getTemplate(
					templateAppId,
					templateId,
				);

				if (!templateBoard) {
					toastError("Template not found", <XIcon />);
					return;
				}

				// Convert template nodes to the format expected by copyPasteCommand
				const templateNodes = Object.values(templateBoard.nodes);
				const templateComments = Object.values(templateBoard.comments);
				const templateLayers = Object.values(templateBoard.layers);
				const templateVariables = Object.values(templateBoard.variables);

				if (
					templateNodes.length === 0 &&
					templateComments.length === 0 &&
					templateLayers.length === 0
				) {
					toastError("Template is empty", <XIcon />);
					return;
				}

				// Create a copy-paste command to insert template contents
				const command = {
					command_type: "CopyPaste" as const,
					original_nodes: templateNodes,
					original_comments: templateComments,
					original_layers: templateLayers,
					original_variables: templateVariables,
					new_nodes: [],
					new_comments: [],
					new_layers: [],
					current_layer: currentLayer,
					offset: [100, 100, 0],
					old_mouse: undefined,
				};

				await executeCommand(command as any);
				setTemplateSelectorOpen(false);
			} catch (error) {
				console.error("Failed to apply template:", error);
				toastError("Failed to apply template", <XIcon />);
			}
		},
		[backend.templateState, executeCommand, currentLayer, version],
	);

	const [varsOpen, setVarsOpen] = useState(false);

	const [flowScriptSheetOpen, setFlowScriptSheetOpen] = useState(false);
	const [flowScriptPanelVisible, setFlowScriptPanelVisible] = useState(false);
	const [runsOpen, setRunsOpen] = useState(false);
	const [logsOpen, setLogsOpen] = useState(false);
	const [logNodeIdFilter, setLogNodeIdFilter] = useState<string | undefined>();
	const [pagesOpen, setPagesOpen] = useState(false);
	const [searchOpen, setSearchOpen] = useState(false);
	const [searchMode, setSearchMode] = useState<"dialog" | "sidebar">("dialog");
	const [copilotOpen, setCopilotOpen] = useState(false);
	const [copilotWorkspaceVisible, setCopilotWorkspaceVisible] = useState(false);
	const [copilotInitialPrompt, setCopilotInitialPrompt] = useState<
		string | undefined
	>();
	// Stable snapshot of the selected flow-node ids handed to FlowPilot. Kept as
	// state (updated only on real selection changes) so the copilot subtree is not
	// re-rendered on every unrelated FlowBoard render.
	const [selectedNodeIds, setSelectedNodeIds] = useState<string[]>([]);

	const handleCopilotClose = useCallback(() => {
		setCopilotOpen(false);
		setCopilotInitialPrompt(undefined);
		setCopilotWorkspaceVisible(false);
	}, []);
	// Single launcher: hosts with a global assistant (desktop) route to the shared surface store,
	// everything else keeps the embedded FlowCopilot panel.
	const openAssistant = useCallback(
		(prompt?: string) => {
			if (externalAssistant) {
				useAssistantSurface.getState().requestOpenAssistant(prompt);
				return;
			}
			if (prompt) setCopilotInitialPrompt(prompt);
			setCopilotOpen(true);
		},
		[externalAssistant],
	);
	const handleClearRunContext = useCallback(
		() => setCurrentMetadata(undefined),
		[setCurrentMetadata],
	);

	useEffect(() => {
		if (!copilotOpen) {
			setCopilotWorkspaceVisible(false);
		}
	}, [copilotOpen]);
	const isMobile = useMediaQuery("(max-width: 767px)");

	const { toggleVars, toggleLogs } = useFlowPanels({
		varPanelRef,
		logPanelRef,
		setVarsOpen,
		setLogsOpen,
	});

	// Runs and FlowScript are collapsible siblings on the right edge of the outer
	// horizontal group. Collapsing one via the per-panel imperative API makes
	// react-resizable-panels hand the freed width to its immediate neighbor, which
	// un-collapses it (e.g. closing FlowScript surfaces Runs). Rewrite the whole
	// layout instead and give the freed width back to the flexible main container.
	const setSidePanelSize = useCallback((index: number, size: number) => {
		const group = panelGroupRef.current;
		if (!group) return;
		const layout = group.getLayout();
		const current = layout[index] ?? 0;
		if (Math.abs(current - size) < 0.01) return;
		const next = [...layout];
		next[index] = size;
		next[MAIN_PANEL_INDEX] = Math.max(
			0,
			(next[MAIN_PANEL_INDEX] ?? 0) - (size - current),
		);
		group.setLayout(next);
	}, []);

	const toggleRunHistory = useCallback(() => {
		if (isMobile) {
			setRunsOpen((v) => !v);
			return;
		}
		const group = panelGroupRef.current;
		if (!group) return;
		const open = (group.getLayout()[RUNS_PANEL_INDEX] ?? 0) < 1;
		setSidePanelSize(RUNS_PANEL_INDEX, open ? 30 : 0);
	}, [isMobile, setSidePanelSize]);

	const togglePages = useCallback(() => {
		if (isMobile) {
			setPagesOpen((v) => !v);
		} else {
			// For desktop, we use the runs panel area to show pages
			// Toggle the runs panel if needed, or just open the sheet
			setPagesOpen((v) => !v);
		}
	}, [isMobile]);

	const toggleFlowScript = useCallback(() => {
		if (isMobile) {
			setFlowScriptSheetOpen((v) => !v);
			return;
		}
		const group = panelGroupRef.current;
		if (!group) return;
		const open = (group.getLayout()[FLOWSCRIPT_PANEL_INDEX] ?? 0) < 1;
		setSidePanelSize(FLOWSCRIPT_PANEL_INDEX, open ? 35 : 0);
	}, [isMobile, setSidePanelSize]);

	// Clear selections when version changes
	useEffect(() => {
		selected.current.clear();
		setNodes((nds) =>
			nds.map((node) => ({
				...node,
				selected: false,
			})),
		);
		setEdges((eds) =>
			eds.map((edge) => ({
				...edge,
				selected: false,
			})),
		);
	}, [version, setNodes, setEdges]);

	const onMoveEnd = useCallback(() => {
		void saveViewport();
	}, [saveViewport]);

	// Get runtime-configured variables from the board
	const runtimeConfiguredVars = useMemo(() => {
		if (!board.data) return [];
		return getRuntimeConfiguredVariables(board.data);
	}, [board.data]);

	// Collect WASM (external) node package IDs from the board
	const wasmPackageIds = useMemo(() => {
		if (!board.data) return [];
		const ids = new Set<string>();
		for (const node of Object.values(board.data.nodes)) {
			if (node.wasm?.package_id) ids.add(node.wasm.package_id);
		}
		for (const layer of Object.values(board.data.layers)) {
			for (const node of Object.values(layer.nodes)) {
				if (node.wasm?.package_id) ids.add(node.wasm.package_id);
			}
		}
		return Array.from(ids);
	}, [board.data]);

	const wasmPackagePermissions = useMemo(() => {
		if (!board.data) return {};
		const perms: Record<string, string[]> = {};
		const collect = (node: INode) => {
			if (!node.wasm?.package_id || !node.wasm.permissions?.length) return;
			const existing = perms[node.wasm.package_id] ?? [];
			for (const p of node.wasm.permissions) {
				if (!existing.includes(p)) existing.push(p);
			}
			perms[node.wasm.package_id] = existing;
		};
		for (const node of Object.values(board.data.nodes)) collect(node);
		for (const layer of Object.values(board.data.layers)) {
			for (const node of Object.values(layer.nodes)) collect(node);
		}
		return perms;
	}, [board.data]);

	// WASM consent dialog state
	const [wasmDialogOpen, setWasmDialogOpen] = useState(false);
	const [wasmConsentResolve, setWasmConsentResolve] = useState<
		((granted: boolean) => void) | null
	>(null);

	const checkWasmConsent = useCallback((): Promise<boolean> => {
		if (wasmPackageIds.length === 0) return Promise.resolve(true);
		try {
			if (localStorage.getItem(`wasm-consent-board-${boardId}`) === "1")
				return Promise.resolve(true);
			if (
				wasmPackageIds.every(
					(id) => localStorage.getItem(`wasm-consent-package-${id}`) === "1",
				)
			)
				return Promise.resolve(true);
		} catch {
			/* ignore */
		}
		return new Promise((resolve) => {
			setWasmConsentResolve(() => resolve);
			setWasmDialogOpen(true);
		});
	}, [wasmPackageIds, boardId]);

	const handleWasmConfirm = useCallback(
		(rememberFor: "none" | "board" | "event" | "package") => {
			if (rememberFor === "package") {
				for (const id of wasmPackageIds) {
					try {
						localStorage.setItem(`wasm-consent-package-${id}`, "1");
					} catch {
						/* ignore */
					}
				}
			} else if (rememberFor === "board") {
				try {
					localStorage.setItem(`wasm-consent-board-${boardId}`, "1");
				} catch {
					/* ignore */
				}
			}
			setWasmDialogOpen(false);
			wasmConsentResolve?.(true);
			setWasmConsentResolve(null);
		},
		[boardId, wasmConsentResolve, wasmPackageIds],
	);

	const handleWasmCancel = useCallback(() => {
		setWasmDialogOpen(false);
		wasmConsentResolve?.(false);
		setWasmConsentResolve(null);
	}, [wasmConsentResolve]);

	// Check if runtime variables need configuration before execution
	// Returns { intercepted: false, runtimeVariables: map } if all configured
	// Returns { intercepted: true } if prompting user for values
	const checkRuntimeVarsAndExecute = useCallback(
		async (
			node: INode,
			payload?: object,
			isRemote?: boolean,
		): Promise<{
			intercepted: boolean;
			runtimeVariables?: Record<string, IVariable>;
		}> => {
			// If no runtime-configured variables or no context, proceed directly
			if (runtimeConfiguredVars.length === 0 || !runtimeVarsContext) {
				return { intercepted: false }; // No interception needed
			}

			// Check if all runtime variables are configured
			const hasAll = await runtimeVarsContext.hasAllValues(
				appId,
				runtimeConfiguredVars.map((v) => v.id),
			);

			if (hasAll) {
				// All configured - build the runtime variables map
				const storedValues = await runtimeVarsContext.getValues(appId);
				const runtimeVariables: Record<string, IVariable> = {};

				for (const variable of runtimeConfiguredVars) {
					// For remote execution, skip secrets
					if (isRemote && variable.secret) continue;

					const storedValue = storedValues.get(variable.id);
					if (storedValue?.value !== undefined) {
						runtimeVariables[variable.id] = {
							...variable,
							default_value: storedValue.value,
						};
					}
				}

				return {
					intercepted: false,
					runtimeVariables:
						Object.keys(runtimeVariables).length > 0
							? runtimeVariables
							: undefined,
				};
			}

			// Need to prompt for configuration
			const existingValues = await runtimeVarsContext.getValues(appId);
			setExistingRuntimeVars(existingValues);
			setPendingExecution({ node, payload, isRemote: isRemote ?? false });
			setRuntimeVarsPromptOpen(true);
			return { intercepted: true }; // Intercepted
		},
		[appId, runtimeConfiguredVars, runtimeVarsContext],
	);

	// Cancel runtime vars prompt
	const handleRuntimeVarsCancel = useCallback(() => {
		setRuntimeVarsPromptOpen(false);
		setPendingExecution(null);
	}, []);

	// Internal execution function (called after runtime vars check)
	const executeBoardInternal = useCallback(
		async (
			node: INode,
			payload?: object,
			skipConsentCheck?: boolean,
			runtimeVariables?: Record<string, IVariable>,
		) => {
			let added = false;
			let runId = "";
			let meta: ILogMetadata | undefined = undefined;
			try {
				meta = await backend.boardState.executeBoard(
					appId,
					boardId,
					{
						id: node.id,
						payload: payload,
						runtime_variables: runtimeVariables,
					},
					true,
					async (id: string) => {
						if (added) return;
						runId = id;
						added = true;
						addRun(id, boardId, [node.id]);
					},
					(update) => {
						const runUpdates = update
							.filter((item) => item.event_type.startsWith("run:"))
							.map((item) => item.payload);
						if (runUpdates.length === 0) return;
						const firstItem = runUpdates[0];
						if (!added) {
							runId = firstItem.runId;
							addRun(firstItem.runId, boardId, [node.id]);
							added = true;
						}

						pushUpdate(firstItem.runId, runUpdates);
					},
					skipConsentCheck,
				);
			} catch (error) {
				console.warn("Failed to execute board", error);

				// Check if this is an OAuth error with missing providers
				const oauthError = error as Error & {
					isOAuthError?: boolean;
					missingProviders?: unknown[];
				};
				if (oauthError.isOAuthError && oauthError.missingProviders) {
					// Emit custom event for OAuth handling
					window.dispatchEvent(
						new CustomEvent("flow:oauth-required", {
							detail: {
								missingProviders: oauthError.missingProviders,
								appId,
								boardId,
								nodeId: node.id,
								payload,
							},
						}),
					);
					return;
				}

				const rpaPermissionError = error as Error & {
					isRpaPermissionError?: boolean;
					permissions?: unknown;
				};
				if (rpaPermissionError.isRpaPermissionError) {
					window.dispatchEvent(
						new CustomEvent("flow:rpa-permissions-required", {
							detail: {
								appId,
								boardId,
								nodeId: node.id,
								payload,
								permissions: rpaPermissionError.permissions,
								skipConsentCheck,
							},
						}),
					);
					return;
				}

				const errorMessage = getErrorMessage(error, "");
				toastError(
					errorMessage || "Failed to execute board",
					<PlayCircleIcon className="w-4 h-4" />,
				);
				return;
			}
			removeRun(runId);
			if (!meta && !runId) {
				toastError(
					"Failed to execute board",
					<PlayCircleIcon className="w-4 h-4" />,
				);
				return;
			}
			await refetchLogs(backend);
			// Find the full metadata from currentLogs by run_id
			// The meta from runBoard may be incomplete for remote executions
			const targetRunId = meta?.run_id || runId;
			const fullMeta = useLogAggregation
				.getState()
				.currentLogs.find((log) => log.run_id === targetRunId);
			if (fullMeta) setCurrentMetadata(fullMeta);
		},
		[
			appId,
			boardId,
			backend,
			refetchLogs,
			pushUpdate,
			addRun,
			removeRun,
			setCurrentMetadata,
		],
	);

	const executeBoardRemoteInternal = useCallback(
		async (
			node: INode,
			payload?: object,
			runtimeVariables?: Record<string, IVariable>,
		) => {
			if (!backend.boardState.executeBoardRemote) {
				toastError(
					"Remote execution not available",
					<PlayCircleIcon className="w-4 h-4" />,
				);
				return;
			}

			let added = false;
			let runId = "";
			let meta: ILogMetadata | undefined = undefined;
			try {
				meta = await backend.boardState.executeBoardRemote(
					appId,
					boardId,
					{
						id: node.id,
						payload: payload,
						runtime_variables: runtimeVariables,
					},
					true,
					async (id: string) => {
						if (added) return;
						runId = id;
						added = true;
						addRun(id, boardId, [node.id]);
					},
					(update) => {
						const runUpdates = update
							.filter((item) => item.event_type.startsWith("run:"))
							.map((item) => item.payload);
						if (runUpdates.length === 0) return;
						const firstItem = runUpdates[0];
						if (!added) {
							runId = firstItem.runId;
							addRun(firstItem.runId, boardId, [node.id]);
							added = true;
						}

						pushUpdate(firstItem.runId, runUpdates);
					},
				);
			} catch (error) {
				console.warn("Failed to execute board remotely", error);
				const errorMessage = getErrorMessage(error, "");
				toastError(
					errorMessage || "Failed to execute board on server",
					<PlayCircleIcon className="w-4 h-4" />,
				);
				return;
			}
			removeRun(runId);
			if (!meta && !runId) {
				toastError(
					"Failed to execute board on server",
					<PlayCircleIcon className="w-4 h-4" />,
				);
				return;
			}
			await refetchLogs(backend);
			// Find the full metadata from currentLogs by run_id
			// The meta from remote execution is incomplete (only has run_id, status, duration_ms)
			const targetRunId = meta?.run_id || runId;
			const fullMeta = useLogAggregation
				.getState()
				.currentLogs.find((log) => log.run_id === targetRunId);
			if (fullMeta) setCurrentMetadata(fullMeta);
		},
		[
			appId,
			boardId,
			backend,
			refetchLogs,
			pushUpdate,
			addRun,
			removeRun,
			setCurrentMetadata,
		],
	);

	// Handle saving runtime variables and continuing execution
	const handleRuntimeVarsSave = useCallback(
		async (values: RuntimeVariableValue[]) => {
			if (!runtimeVarsContext || !pendingExecution || !board.data) return;

			// Save the values
			await runtimeVarsContext.saveValues(
				appId,
				boardId,
				values.map((v) => {
					const variable = runtimeConfiguredVars.find(
						(rv) => rv.id === v.variableId,
					);
					return {
						variableId: v.variableId,
						variableName: variable?.name ?? "",
						value: v.value,
						isSecret: variable?.secret ?? false,
					};
				}),
			);

			// Build runtime variables map from the saved values
			const runtimeVariables: Record<string, IVariable> = {};
			for (const v of values) {
				const variable = runtimeConfiguredVars.find(
					(rv) => rv.id === v.variableId,
				);
				if (variable) {
					// For remote execution, skip secrets
					if (pendingExecution.isRemote && variable.secret) continue;

					runtimeVariables[variable.id] = {
						...variable,
						default_value: v.value,
					};
				}
			}

			// Close prompt and proceed with execution
			setRuntimeVarsPromptOpen(false);
			const { node, payload, isRemote } = pendingExecution;
			setPendingExecution(null);

			const varsMap =
				Object.keys(runtimeVariables).length > 0 ? runtimeVariables : undefined;
			if (isRemote) {
				await executeBoardRemoteInternal(node, payload, varsMap);
			} else {
				await executeBoardInternal(node, payload, true, varsMap);
			}
		},
		[
			appId,
			boardId,
			runtimeVarsContext,
			pendingExecution,
			runtimeConfiguredVars,
			board.data,
			executeBoardInternal,
			executeBoardRemoteInternal,
		],
	);

	// Public execution function - checks WASM consent + runtime vars first
	const executeBoard = useCallback(
		async (node: INode, payload?: object, skipConsentCheck?: boolean) => {
			const wasmOk = await checkWasmConsent();
			if (!wasmOk) return;
			const result = await checkRuntimeVarsAndExecute(node, payload, false);
			if (!result.intercepted) {
				await executeBoardInternal(
					node,
					payload,
					skipConsentCheck,
					result.runtimeVariables,
				);
			}
		},
		[checkWasmConsent, checkRuntimeVarsAndExecute, executeBoardInternal],
	);

	// Public remote execution function - checks WASM consent + runtime vars first
	const executeBoardRemote = useCallback(
		async (node: INode, payload?: object) => {
			const wasmOk = await checkWasmConsent();
			if (!wasmOk) return;
			const result = await checkRuntimeVarsAndExecute(node, payload, true);
			if (!result.intercepted) {
				await executeBoardRemoteInternal(
					node,
					payload,
					result.runtimeVariables,
				);
			}
		},
		[checkWasmConsent, checkRuntimeVarsAndExecute, executeBoardRemoteInternal],
	);

	// Listen for OAuth retry events to re-execute after authorization
	useEffect(() => {
		const handleOAuthRetry = (event: Event) => {
			const retryEvent = event as CustomEvent<{
				appId: string;
				boardId: string;
				nodeId: string;
				payload?: object;
				skipConsentCheck?: boolean;
			}>;

			const {
				appId: eventAppId,
				boardId: eventBoardId,
				nodeId,
				payload,
				skipConsentCheck,
			} = retryEvent.detail;

			// Only handle if this is for our board
			if (eventAppId !== appId || eventBoardId !== boardId) return;

			// Find the node and re-execute
			const node = nodes.find((n) => n.id === nodeId);
			if (node?.data?.node) {
				executeBoard(node.data.node as INode, payload, skipConsentCheck);
			} else {
				console.warn("[FlowBoard] Node not found for OAuth retry:", nodeId);
			}
		};

		window.addEventListener("flow:oauth-retry", handleOAuthRetry);
		return () => {
			window.removeEventListener("flow:oauth-retry", handleOAuthRetry);
		};
	}, [appId, boardId, nodes, executeBoard]);

	useEffect(() => {
		const handleRpaPermissionsRetry = (event: Event) => {
			const retryEvent = event as CustomEvent<{
				appId: string;
				boardId: string;
				nodeId: string;
				payload?: object;
				skipConsentCheck?: boolean;
			}>;

			const {
				appId: eventAppId,
				boardId: eventBoardId,
				nodeId,
				payload,
				skipConsentCheck,
			} = retryEvent.detail;

			if (eventAppId !== appId || eventBoardId !== boardId) return;

			const node = nodes.find((n) => n.id === nodeId);
			if (node?.data?.node) {
				executeBoard(node.data.node as INode, payload, skipConsentCheck);
			} else {
				console.warn("[FlowBoard] Node not found for RPA retry:", nodeId);
			}
		};

		window.addEventListener(
			"flow:rpa-permissions-retry",
			handleRpaPermissionsRetry,
		);
		return () => {
			window.removeEventListener(
				"flow:rpa-permissions-retry",
				handleRpaPermissionsRetry,
			);
		};
	}, [appId, boardId, nodes, executeBoard]);

	// Listen for external refetch requests (e.g., after recording insertion)
	useEffect(() => {
		const handleRefetchBoard = () => {
			board.refetch();
		};
		window.addEventListener("flow:refetch-board", handleRefetchBoard);
		return () => {
			window.removeEventListener("flow:refetch-board", handleRefetchBoard);
		};
	}, [board]);

	const handlePasteCB = useCallback(
		async (event: ClipboardEvent) => {
			if (shouldIgnoreBoardClipboardEvent(event)) {
				return;
			}
			if (typeof version !== "undefined") {
				toastError("Cannot change old version", <XIcon />);
				return;
			}
			const mp = mousePositionRef.current;
			const currentCursorPosition = screenToFlowPosition({
				x: mp.x,
				y: mp.y,
			});

			// Try to handle media paste first (images/videos)
			const wasMediaPaste = await handleMediaPaste(
				event,
				currentCursorPosition,
			);
			if (wasMediaPaste) return;

			// Fall back to regular paste handling
			await handlePaste(
				event,
				currentCursorPosition,
				boardId,
				executeCommand,
				currentLayer,
				catalog.data ?? undefined,
			);
		},
		[
			boardId,
			executeCommand,
			currentLayer,
			version,
			handleMediaPaste,
			catalog.data,
		],
	);

	const handleCopyCB = useCallback(
		(event?: ClipboardEvent) => {
			if (shouldIgnoreBoardClipboardEvent(event)) {
				return;
			}
			if (!board.data) return;
			const mp = mousePositionRef.current;
			const currentCursorPosition = screenToFlowPosition({
				x: mp.x,
				y: mp.y,
			});
			handleCopy(nodes, board.data, currentCursorPosition, event, currentLayer);
		},
		[nodes, board.data, currentLayer],
	);

	const openNodeInfo = useCallback((node: INode) => {
		nodeInfoOverlayRef.current?.openNodeInfo(node);
	}, []);

	const handleExplainNodes = useCallback(
		(nodeIds: string[]) => {
			// Select the nodes for context
			const nodeIdSet = new Set(nodeIds);
			setNodes((nds) =>
				nds.map((node) => ({
					...node,
					selected: nodeIdSet.has(node.id),
				})),
			);
			selected.current = nodeIdSet;

			// Build the explain prompt
			const nodeCount = nodeIds.length;
			const prompt =
				nodeCount === 1
					? "Explain what this node does and how it works in the context of this flow."
					: `Explain what these ${nodeCount} selected nodes do and how they work together in this flow.`;

			openAssistant(prompt);
		},
		[setNodes, openAssistant],
	);

	const placeNode = useCallback(
		async (node: INode, position?: { x: number; y: number }) => {
			const location = screenToFlowPosition({
				x: position?.x ?? clickPosition.x,
				y: position?.y ?? clickPosition.y,
			});

			await handlePlaceNode({
				node,
				position: location,
				droppedPin,
				currentLayer,
				refs: board.data?.refs ?? {},
				boardNodes: board.data?.nodes ?? {},
				pinCache,
				executeCommand,
			});
		},
		[
			clickPosition,
			droppedPin,
			board.data?.refs,
			board.data?.nodes,
			currentLayer,
			screenToFlowPosition,
			pinCache,
			executeCommand,
		],
	);

	const placeNodeShortcut = useCallback(
		async (node: INode) => {
			const mp = mousePositionRef.current;
			await placeNode(node, {
				x: mp.x,
				y: mp.y,
			});
		},
		[placeNode],
	);

	const placePlaceholder = useCallback(
		async (name: string, position?: { x: number; y: number }) => {
			const delayNode = catalog.data?.find((node) => node.name === "delay");
			const location = screenToFlowPosition({
				x: position?.x ?? clickPosition.x,
				y: position?.y ?? clickPosition.y,
			});

			await handlePlacePlaceholder({
				name,
				position: location,
				droppedPin,
				currentLayer,
				refs: board.data?.refs ?? {},
				pinCache,
				delayNode,
				executeCommand,
				executeCommands,
			});
		},
		[
			clickPosition,
			droppedPin,
			board.data?.refs,
			executeCommand,
			executeCommands,
			pinCache,
			currentLayer,
			screenToFlowPosition,
			catalog.data,
		],
	);

	const deleteSelectedElements = useCallback(async () => {
		if (
			deleteSelectionInFlightRef.current ||
			typeof version !== "undefined" ||
			!board.data
		) {
			return;
		}

		const selectedNodeIds = new Set(
			getNodes()
				.filter((node) => node.selected)
				.map((node) => node.id),
		);
		const selectedEdgeIds = new Set(
			edges.filter((edge) => edge.selected).map((edge) => edge.id),
		);

		if (selectedNodeIds.size === 0 && selectedEdgeIds.size === 0) {
			return;
		}

		const isHandleOwnedBySelectedNode = (handleId: string | undefined) => {
			if (!handleId) {
				return false;
			}

			if (handleId.startsWith("ref_in_")) {
				return selectedNodeIds.has(handleId.replace("ref_in_", ""));
			}

			if (handleId.startsWith("ref_out_")) {
				return selectedNodeIds.has(handleId.replace("ref_out_", ""));
			}

			const [, pinOwner] = pinCache.get(handleId) || [];
			return pinOwner ? selectedNodeIds.has(pinOwner.id) : false;
		};

		const commands: IGenericCommand[] = [];

		for (const node of getNodes().filter(
			(currentNode) => currentNode.selected,
		)) {
			if (node.data?.node) {
				commands.push(
					removeNodeCommand({
						node: node.data.node as INode,
						connected_nodes: [],
					}),
				);
				continue;
			}

			if (node.data?.comment) {
				commands.push(
					removeCommentCommand({
						comment: node.data.comment as IComment,
					}),
				);
				continue;
			}

			if (node.type === "layerNode" && node.data?.layer) {
				commands.push(
					removeLayerCommand({
						child_layers: [],
						layer: node.data.layer as ILayer,
						layer_nodes: [],
						layers: [],
						nodes: [],
						preserve_nodes: false,
					}),
				);
			}
		}

		for (const edge of edges.filter((currentEdge) => currentEdge.selected)) {
			const functionReferenceNodeIds =
				getFunctionReferenceNodeIdsFromEdge(edge);
			if (functionReferenceNodeIds) {
				if (
					selectedNodeIds.has(functionReferenceNodeIds.refOutNodeId) ||
					selectedNodeIds.has(functionReferenceNodeIds.refInNodeId)
				) {
					continue;
				}

				const command = removeFunctionReferenceCommandForEdge({
					edge,
					boardNodes: board.data.nodes,
				});
				if (command) commands.push(command);
				continue;
			}

			if (
				isHandleOwnedBySelectedNode(edge.sourceHandle) ||
				isHandleOwnedBySelectedNode(edge.targetHandle)
			) {
				continue;
			}

			const [fromPin, fromNode] = pinCache.get(edge.sourceHandle ?? "") || [];
			const [toPin, toNode] = pinCache.get(edge.targetHandle ?? "") || [];

			if (!fromPin || !fromNode || !toPin || !toNode) {
				continue;
			}

			commands.push(
				disconnectPinsCommand({
					from_node: fromNode.id,
					from_pin: fromPin.id,
					to_node: toNode.id,
					to_pin: toPin.id,
				}),
			);
		}

		if (commands.length === 0) {
			return;
		}

		deleteSelectionInFlightRef.current = true;
		try {
			selected.current.clear();
			await executeCommands(commands);
		} finally {
			deleteSelectionInFlightRef.current = false;
		}
	}, [board.data, edges, executeCommands, getNodes, pinCache, version]);

	useKeyboardShortcuts({
		board,
		catalog,
		version,
		appId,
		boardId,
		mousePositionRef,
		onDeleteSelection: deleteSelectedElements,
		placeNode,
		undo,
		redo,
		rollbackUndo,
		rollbackRedo,
		stampHistory,
	});

	useEffect(() => {
		const handleRemoteBoardApplied = async (event: Event): Promise<void> => {
			const detail = (event as CustomEvent<{ appId: string; boardId: string }>)
				.detail;

			if (!detail || detail.appId !== appId || detail.boardId !== boardId) {
				return;
			}

			await clearHistory();
		};

		window.addEventListener(
			REMOTE_BOARD_APPLIED_EVENT,
			handleRemoteBoardApplied as EventListener,
		);

		return () => {
			window.removeEventListener(
				REMOTE_BOARD_APPLIED_EVENT,
				handleRemoteBoardApplied as EventListener,
			);
		};
	}, [appId, boardId, clearHistory]);

	const handleDrop = useCallback(
		async (event: any) => {
			const { type, screenPosition } = event.detail;

			// Function layer drop -> place a CallFunction node
			if (type === "function-layer") {
				const layerId: string = event.detail.layerId;
				const callFnNode = catalog.data?.find(
					(node) => node.name === "control_call_function",
				);
				if (!callFnNode) return;

				const layerPin = Object.values(callFnNode.pins).find(
					(pin) => pin.name === "function_layer_id",
				);
				if (!layerPin) return;

				layerPin.default_value = convertJsonToUint8Array(layerId);
				callFnNode.pins[layerPin.id] = layerPin;

				placeNode(callFnNode, {
					x: screenPosition.x,
					y: screenPosition.y,
				});
				return;
			}

			// Variable drop -> place a Get/Set variable node
			const variable: IVariable = event.detail.variable;
			const operation: "set" | "get" = event.detail.operation;
			const getVarNode = catalog.data?.find(
				(node) => node.name === `variable_${operation}`,
			);
			if (!getVarNode) return;

			const varRefPin = Object.values(getVarNode.pins).find(
				(pin) => pin.name === "var_ref",
			);
			if (!varRefPin) return;

			varRefPin.default_value = convertJsonToUint8Array(variable.id);
			getVarNode.pins[varRefPin.id] = varRefPin;

			placeNode(getVarNode, {
				x: screenPosition.x,
				y: screenPosition.y,
			});
		},
		[catalog.data, clickPosition, boardId, droppedPin, placeNode],
	);

	const handleCopyRef = useRef(handleCopyCB);
	handleCopyRef.current = handleCopyCB;
	const handlePasteRef = useRef(handlePasteCB);
	handlePasteRef.current = handlePasteCB;

	useEffect(() => {
		const onCopy = (e: Event) => handleCopyRef.current(e as ClipboardEvent);
		const onPaste = (e: Event) => handlePasteRef.current(e as ClipboardEvent);
		document.addEventListener("copy", onCopy);
		document.addEventListener("paste", onPaste);
		return () => {
			document.removeEventListener("copy", onCopy);
			document.removeEventListener("paste", onPaste);
		};
	}, []);

	// Keyboard shortcut: Cmd/Ctrl+Shift+P to toggle pages panel
	useEffect(() => {
		const handleKeyDown = (e: KeyboardEvent) => {
			if (
				(e.metaKey || e.ctrlKey) &&
				e.shiftKey &&
				e.key.toLowerCase() === "p"
			) {
				e.preventDefault();
				setPagesOpen((v) => !v);
			}
		};
		document.addEventListener("keydown", handleKeyDown);
		return () => document.removeEventListener("keydown", handleKeyDown);
	}, []);

	// Keyboard shortcut: Cmd/Ctrl+F to open search dialog, Cmd/Ctrl+Shift+F for sidebar
	useEffect(() => {
		const handleKeyDown = (e: KeyboardEvent) => {
			if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
				e.preventDefault();
				if (e.shiftKey) {
					setSearchMode("sidebar");
					setSearchOpen((prev) => !prev);
				} else {
					setSearchMode("dialog");
					setSearchOpen(true);
				}
			}
		};
		document.addEventListener("keydown", handleKeyDown);
		return () => document.removeEventListener("keydown", handleKeyDown);
	}, []);

	useEffect(() => {
		document.addEventListener("flow-drop", handleDrop);
		return () => {
			document.removeEventListener("flow-drop", handleDrop);
		};
	}, [handleDrop]);

	useEffect(() => {
		const handler = (event: MouseEvent) => {
			mousePositionRef.current = { x: event.clientX, y: event.clientY };
		};
		document.addEventListener("mousemove", handler);
		return () => {
			document.removeEventListener("mousemove", handler);
		};
	}, []);

	// Build O(1) lookup sets for marking unavailable nodes:
	// - nodeNames:  built-in (non-WASM) node names
	// - wasmNodeKeys: "package_id:node_name" keys for WASM nodes
	const catalogLookup = useMemo(() => {
		if (!catalog.data) return undefined;
		const nodeNames = new Set<string>();
		const wasmNodeKeys = new Set<string>();
		for (const n of catalog.data) {
			if (n.wasm?.package_id) {
				wasmNodeKeys.add(`${n.wasm.package_id}:${n.name}`);
			} else {
				nodeNames.add(n.name);
			}
		}
		return { nodeNames, wasmNodeKeys };
	}, [catalog.data]);

	// Refs for callbacks used in parseBoard to avoid re-running on every callback identity change
	const executeBoardRef = useRef(executeBoard);
	executeBoardRef.current = executeBoard;
	const executeBoardRemoteRef = useRef(executeBoardRemote);
	executeBoardRemoteRef.current = executeBoardRemote;
	const executeCommandRef = useRef(executeCommand);
	executeCommandRef.current = executeCommand;
	const pushLayerRef = useRef(pushLayer);
	pushLayerRef.current = pushLayer;
	const openNodeInfoRef = useRef(openNodeInfo);
	openNodeInfoRef.current = openNodeInfo;
	const handleExplainNodesRef = useRef(handleExplainNodes);
	handleExplainNodesRef.current = handleExplainNodes;

	const handleFilterLogs = useCallback((nodeId: string) => {
		setLogNodeIdFilter(nodeId);
		if (
			typeof window !== "undefined" &&
			window.matchMedia("(max-width: 767px)").matches
		) {
			setLogsOpen(true);
			return;
		}
		if (!logPanelRef.current) return;
		logPanelRef.current.expand();
		const size = logPanelRef.current.getSize();
		if (size < 10) logPanelRef.current.resize(20);
	}, []);
	const handleFilterLogsRef = useRef(handleFilterLogs);
	handleFilterLogsRef.current = handleFilterLogs;

	// Extract stable primitives from complex objects to avoid re-parsing on unrelated changes
	const connectionMode =
		currentProfile.data?.settings?.connection_mode ?? "default";
	const isOffline = app.data?.visibility === IAppVisibility.Offline;
	const hasRemoteExecution = !!backend.boardState.executeBoardRemote;

	useEffect(() => {
		if (!board.data) return;
		boardRef.current = board.data;

		const parsed = parseBoard(
			board.data,
			appId,
			handleCopyCB,
			(...args: Parameters<typeof pushLayer>) => pushLayerRef.current(...args),
			(...args: Parameters<typeof executeBoard>) =>
				executeBoardRef.current(...args),
			(...args: Parameters<typeof executeCommand>) =>
				executeCommandRef.current(...args),
			selected.current,
			connectionMode,
			nodes,
			edges,
			currentLayer,
			boardRef,
			version,
			(node: INode) => openNodeInfoRef.current(node),
			(nodeIds: string[]) => handleExplainNodesRef.current(nodeIds),
			(nodeId: string) => handleFilterLogsRef.current(nodeId),
			hasRemoteExecution
				? {
						isOffline,
						onRemoteExecute: (node: INode, payload?: object) =>
							executeBoardRemoteRef.current(node, payload),
					}
				: undefined,
			catalogLookup,
			selectorDataRef,
			selectorDataVersion,
		);

		setNodes(parsed.nodes);
		setEdges(parsed.edges);
		setPinCache(parsed.cache);
	}, [
		board.data,
		currentLayer,
		connectionMode,
		version,
		isOffline,
		hasRemoteExecution,
		catalogLookup,
		selectorDataVersion,
	]);

	// Apply remote execution presence indicators to nodes
	const remoteExecRef = useRef<Set<string>>(new Set());
	useEffect(() => {
		const prev = remoteExecRef.current;
		const next = remoteExecutingNodeIds;
		// Check if anything actually changed
		if (prev.size === next.size && [...next].every((id) => prev.has(id)))
			return;
		remoteExecRef.current = next;

		setNodes((nds: any) => {
			if (nds.length === 0) return nds;
			return nds.map((node: any) => {
				if (node.type !== "node" && node.type !== "callFunctionNode")
					return node;
				const isRemoteExec = next.has(node.id);
				const wasRemoteExec = !!node.data.remoteExecuting;
				if (isRemoteExec === wasRemoteExec) return node;
				return {
					...node,
					data: { ...node.data, remoteExecuting: isRemoteExec || undefined },
				};
			});
		});
	}, [remoteExecutingNodeIds, setNodes]);

	// Inject peerUsers map into node data so nodes can display avatars for remote
	// selections. peerUsers now has a stable identity (see usePeerUserInfo), so this
	// runs only when peer user content actually changes — not on every render — and
	// returns the same nodes array when nothing changed so ReactFlow doesn't reconcile.
	const peerUsersRef = useRef(peerUsers);
	peerUsersRef.current = peerUsers;
	useEffect(() => {
		setNodes((nds: any) => {
			if (nds.length === 0) return nds;
			let changed = false;
			const next = nds.map((node: any) => {
				if (node.type !== "node" && node.type !== "callFunctionNode")
					return node;
				if (node.data.peerUsers === peerUsers) return node;
				changed = true;
				return { ...node, data: { ...node.data, peerUsers } };
			});
			return changed ? next : nds;
		});
	}, [peerUsers, setNodes]);

	const nodeTypes = useMemo(
		() => ({
			flowNode: FlowNode,
			commentNode: CommentNode,
			mediaNode: MediaNode,
			uploadPlaceholderNode: UploadPlaceholderNode,
			layerNode: LayerNode,
			layerInnerNode: LayerInnerNode,
			callFunctionNode: CallFunctionNode,
			node: FlowNode,
		}),
		[],
	);

	const edgeTypes = useMemo(
		() => ({
			veil: FlowVeilEdge,
			execution: FlowExecutionEdge,
			data: FlowDataEdge,
		}),
		[],
	);

	const miniMapNodeColor = useCallback((node: Node) => {
		if (node.type === "layerNode")
			return "color-mix(in oklch, var(--foreground) 50%, transparent)";

		if (node.type === "node") {
			const nodeData: INode = node.data.node as INode;
			if (nodeData.event_callback)
				return "color-mix(in oklch, var(--primary) 80%, transparent)";
			if (nodeData.start)
				return "color-mix(in oklch, var(--primary) 80%, transparent)";
			if (
				!Object.values(nodeData.pins).find(
					(pin) => pin.data_type === IVariableType.Execution,
				)
			) {
				return "color-mix(in oklch, var(--tertiary) 80%, transparent)";
			}
			return "color-mix(in oklch, var(--muted) 80%, transparent)";
		}
		if (node.type === "commentNode") {
			const commentData: IComment = node.data.comment as IComment;
			let color =
				commentData.color ??
				"color-mix(in oklch, var(--muted) 80%, transparent)";

			if (color.startsWith("#")) {
				color = hexToRgba(color, 0.3);
			}
			return color;
		}
		return "color-mix(in oklch, var(--primary) 60%, transparent)";
	}, []);

	const onConnect = useCallback(
		(params: any) =>
			setEdges((eds) =>
				handleConnection({
					params,
					version,
					boardNodes: board.data?.nodes ?? {},
					pinCache,
					executeCommand,
					addEdge: (p: any, e: any[]) => addEdge(p, e),
					currentEdges: eds,
				}),
			),
		[setEdges, pinCache, version, executeCommand, board.data?.nodes],
	);

	const onSelectionChange = useCallback<OnSelectionChangeFunc<Node, Edge>>(
		({ nodes: selectedNodes }) => {
			const nodeIds = selectedNodes
				.filter(
					(selectedNode) =>
						selectedNode.type === "node" ||
						selectedNode.type === "callFunctionNode",
				)
				.map((selectedNode) => selectedNode.id);
			setSelectedNodeIds((prev) => {
				if (prev.length !== nodeIds.length) return nodeIds;
				const prevSet = new Set(prev);
				return nodeIds.every((id) => prevSet.has(id)) ? prev : nodeIds;
			});
			if (!awareness) return;
			awareness.setLocalStateField("selection", { nodes: nodeIds });
			// Broadcast active node when user clicks a single node
			if (nodeIds.length === 1) {
				broadcastActiveNode(nodeIds[0]);
			} else {
				broadcastActiveNode(undefined);
			}
		},
		[awareness, broadcastActiveNode],
	);

	const selectNodes = useCallback(
		(nodeIds: string[]) => {
			const nodeIdSet = new Set(nodeIds);
			setNodes((nds) =>
				nds.map((node) => ({
					...node,
					selected: nodeIdSet.has(node.id),
				})),
			);
			selected.current = nodeIdSet;
		},
		[setNodes],
	);

	const onConnectEnd = useCallback(
		(
			event: MouseEvent | TouchEvent,
			connectionState: FinalConnectionState<InternalNode>,
		) => {
			// when a connection is dropped on the pane it's not valid
			if (!connectionState.isValid) {
				// we need to remove the wrapper bounds, in order to get the correct position

				const { clientX, clientY } =
					"changedTouches" in event ? event.changedTouches[0] : event;

				const handle = connectionState.fromHandle;
				if (handle?.id) {
					// Check if this is a function reference handle
					if (
						handle.id.startsWith("ref_in_") ||
						handle.id.startsWith("ref_out_")
					) {
						// Create a synthetic pin object for ref handles
						const syntheticPin: IPin = {
							id: handle.id,
							name: handle.id.startsWith("ref_in_") ? "ref_in" : "ref_out",
							friendly_name: handle.id.startsWith("ref_in_")
								? "Function Reference In"
								: "Function Reference Out",
							pin_type: handle.id.startsWith("ref_in_")
								? IPinType.Input
								: IPinType.Output,
							data_type: IVariableType.Generic,
							value_type: IValueType.Normal,
							depends_on: [],
							connected_to: [],
							index: 0,
							description: "",
							schema: "",
						};
						setDroppedPin(syntheticPin);
					} else {
						const [pin, _node] = pinCache.get(handle.id) || [];
						setDroppedPin(pin);
					}
				}

				const contextMenuEvent = new MouseEvent("contextmenu", {
					bubbles: true,
					cancelable: true,
					view: window,
					clientX,
					clientY,
				});

				flowRef.current?.dispatchEvent(contextMenuEvent);
			}
		},
		[pinCache],
	);

	const onNodesChangeIntercept: OnNodesChange = useCallback(
		(changes: any[]) =>
			setNodes((nds) =>
				handleNodesChange({
					changes,
					currentNodes: nds,
					selected,
					version,
					boardData: board.data,
					deletingNodesRef,
					executeCommands,
					applyNodeChanges,
				}),
			),
		[setNodes, board.data, executeCommands, version],
	);

	const onEdgesChange: OnEdgesChange = useCallback(
		(changes: any[]) =>
			setEdges((eds) =>
				handleEdgesChange({
					changes,
					currentEdges: eds,
					selected,
					version,
					boardData: board.data,
					pinCache,
					deletingNodesRef,
					executeCommands,
					applyEdgeChanges,
				}),
			),
		[setEdges, board.data, pinCache, executeCommands, version],
	);

	const onReconnectStart = useCallback(() => {
		edgeReconnectSuccessful.current = false;
	}, []);

	const onReconnect = useCallback(
		async (oldEdge: any, newConnection: Connection) => {
			// Don't execute commands when viewing an old version
			if (typeof version !== "undefined") {
				return;
			}

			// Check if the edge is actually being moved
			const new_id = `${newConnection.sourceHandle}-${newConnection.targetHandle}`;
			if (oldEdge.id === new_id) {
				return;
			}

			// Check if this is a veil edge (fn_ref) FIRST - handle it differently
			const isOldRefConnection =
				(oldEdge.sourceHandle?.startsWith("ref_out_") &&
					oldEdge.targetHandle?.startsWith("ref_in_")) ||
				(oldEdge.sourceHandle?.startsWith("ref_in_") &&
					oldEdge.targetHandle?.startsWith("ref_out_"));
			const isNewRefConnection =
				(newConnection.sourceHandle?.startsWith("ref_out_") &&
					newConnection.targetHandle?.startsWith("ref_in_")) ||
				(newConnection.sourceHandle?.startsWith("ref_in_") &&
					newConnection.targetHandle?.startsWith("ref_out_"));

			if (isOldRefConnection && isNewRefConnection) {
				const oldSource = oldEdge.sourceHandle;
				const oldTarget = oldEdge.targetHandle;
				const newSource = newConnection.sourceHandle;
				const newTarget = newConnection.targetHandle;

				// Determine which end was reconnected
				const sourceChanged = oldSource !== newSource;
				const targetChanged = oldTarget !== newTarget;

				const commands: IGenericCommand[] = [];

				if (sourceChanged) {
					// Source (ref_out) was reconnected - update both old and new source nodes
					const oldRefOutNodeId = oldSource?.replace("ref_out_", "") || "";
					const newRefOutNodeId = newSource?.replace("ref_out_", "") || "";
					const refInNodeId = oldTarget?.replace("ref_in_", "") || "";

					const oldRefOutNode = board.data?.nodes[oldRefOutNodeId];
					const newRefOutNode = board.data?.nodes[newRefOutNodeId];

					// Remove ref from old source node
					if (oldRefOutNode && refInNodeId) {
						const currentRefs = oldRefOutNode.fn_refs?.fn_refs ?? [];
						const updatedRefs = currentRefs.filter(
							(ref: string) => ref !== refInNodeId,
						);

						const updatedOldNode = {
							...oldRefOutNode,
							fn_refs: {
								...oldRefOutNode.fn_refs,
								fn_refs: updatedRefs,
								can_reference_fns:
									oldRefOutNode.fn_refs?.can_reference_fns ?? false,
								can_be_referenced_by_fns:
									oldRefOutNode.fn_refs?.can_be_referenced_by_fns ?? false,
							},
						};

						commands.push(updateNodeCommand({ node: updatedOldNode }));
					}

					// Add ref to new source node
					if (newRefOutNode && refInNodeId) {
						const currentRefs = newRefOutNode.fn_refs?.fn_refs ?? [];
						const updatedRefs = [...currentRefs];

						if (!updatedRefs.includes(refInNodeId)) {
							updatedRefs.push(refInNodeId);
						}

						const updatedNewNode = {
							...newRefOutNode,
							fn_refs: {
								...newRefOutNode.fn_refs,
								fn_refs: updatedRefs,
								can_reference_fns:
									newRefOutNode.fn_refs?.can_reference_fns ?? false,
								can_be_referenced_by_fns:
									newRefOutNode.fn_refs?.can_be_referenced_by_fns ?? false,
							},
						};

						commands.push(updateNodeCommand({ node: updatedNewNode }));
					}
				} else if (targetChanged) {
					// Target (ref_in) was reconnected - update the source node's refs
					const refOutNodeId = oldSource?.replace("ref_out_", "") || "";
					const oldRefInNodeId = oldTarget?.replace("ref_in_", "") || "";
					const newRefInNodeId = newTarget?.replace("ref_in_", "") || "";

					const refOutNode = board.data?.nodes[refOutNodeId];

					if (refOutNode && newRefInNodeId && oldRefInNodeId) {
						const currentRefs = refOutNode.fn_refs?.fn_refs ?? [];

						// Remove old ref, add new ref
						const updatedRefs = currentRefs.filter(
							(ref: string) => ref !== oldRefInNodeId,
						);

						if (!updatedRefs.includes(newRefInNodeId)) {
							updatedRefs.push(newRefInNodeId);
						}

						const updatedNode = {
							...refOutNode,
							fn_refs: {
								...refOutNode.fn_refs,
								fn_refs: updatedRefs,
								can_reference_fns:
									refOutNode.fn_refs?.can_reference_fns ?? false,
								can_be_referenced_by_fns:
									refOutNode.fn_refs?.can_be_referenced_by_fns ?? false,
							},
						};

						commands.push(updateNodeCommand({ node: updatedNode }));
					}
				}

				if (commands.length > 0) {
					await executeCommands(commands);
				}
			} else {
				// Regular pin connection reconnection - need to look up nodes
				const oldEdgeToNode = pinToNode(oldEdge.targetHandle);
				const oldEdgeFromNode = pinToNode(oldEdge.sourceHandle);

				if (!oldEdgeToNode || !oldEdgeFromNode) {
					return;
				}

				const commands = [];

				const disconnectCommand = disconnectPinsCommand({
					from_node: oldEdgeFromNode.id,
					from_pin: oldEdge.sourceHandle,
					to_node: oldEdgeToNode.id,
					to_pin: oldEdge.targetHandle,
				});

				commands.push(disconnectCommand);

				if (newConnection.targetHandle && newConnection.sourceHandle) {
					const newConnectionSourceNode = pinToNode(newConnection.sourceHandle);
					const newConnectionTargetNode = pinToNode(newConnection.targetHandle);

					if (newConnectionSourceNode && newConnectionTargetNode)
						commands.push(
							connectPinsCommand({
								from_node: newConnectionSourceNode.id,
								from_pin: newConnection.sourceHandle,
								to_node: newConnectionTargetNode.id,
								to_pin: newConnection.targetHandle,
							}),
						);
				}

				await executeCommands(commands);
			}

			edgeReconnectSuccessful.current = true;
			setEdges((els) => reconnectEdge(oldEdge, newConnection, els));
		},
		[
			setEdges,
			pinToNode,
			executeCommands,
			executeCommand,
			board.data?.nodes,
			version,
		],
	);

	const onScreenshot = useCallback(async () => {
		const viewportEl = document.querySelector(
			".react-flow__viewport",
		) as HTMLElement | null;
		if (!viewportEl) return;

		const nodes = getNodes();
		if (nodes.length === 0) {
			toastError("No nodes to capture", <XIcon />);
			return;
		}

		const nodesBounds = getNodesBounds(nodes);
		const padding = 50;
		const imageWidth = Math.min(4096, nodesBounds.width + padding * 2);
		const imageHeight = Math.min(4096, nodesBounds.height + padding * 2);

		const viewport = getViewportForBounds(
			nodesBounds,
			imageWidth,
			imageHeight,
			0.5,
			2,
			padding,
		);

		const { toPng } = await import("html-to-image");

		try {
			const dataUrl = await toPng(viewportEl, {
				backgroundColor: "transparent",
				width: imageWidth,
				height: imageHeight,
				style: {
					width: `${imageWidth}px`,
					height: `${imageHeight}px`,
					transform: `translate(${viewport.x}px, ${viewport.y}px) scale(${viewport.zoom})`,
				},
				// Skip images that fail to load (cross-origin issues)
				filter: (node) => {
					// Skip video elements as they can't be captured
					if (node instanceof HTMLVideoElement) return false;
					return true;
				},
				// Handle image fetch errors gracefully
				skipFonts: true,
				imagePlaceholder:
					"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
			});

			const response = await fetch(dataUrl);
			const blob = await response.blob();

			try {
				await navigator.clipboard.write([
					new ClipboardItem({ "image/png": blob }),
				]);
				toastSuccess("Screenshot copied to clipboard", <CheckIcon />);
			} catch {
				const link = document.createElement("a");
				link.download = "flow-screenshot.png";
				link.href = dataUrl;
				link.click();
				toastSuccess("Screenshot downloaded", <CheckIcon />);
			}
		} catch (error) {
			console.error("Screenshot failed:", error);
			toastError("Failed to capture screenshot", <XIcon />);
		}
	}, [getNodes]);

	const onReconnectEnd = useCallback(
		async (event: any, edge: any) => {
			// Don't execute commands when viewing an old version
			if (typeof version !== "undefined") {
				return;
			}

			if (!edgeReconnectSuccessful.current) {
				const { source, target, sourceHandle, targetHandle } = edge;
				const functionReferenceCommand = removeFunctionReferenceCommandForEdge({
					edge: { id: edge.id, source, sourceHandle, target, targetHandle },
					boardNodes: board.data?.nodes,
				});
				if (functionReferenceCommand) {
					await executeCommand(functionReferenceCommand);
					setEdges((eds) => eds.filter((e) => e.id !== edge.id));
					edgeReconnectSuccessful.current = true;
					return;
				}

				const from_node = pinToNode(sourceHandle);
				const to_node = pinToNode(targetHandle);
				if (!from_node || !to_node) return;
				const command = disconnectPinsCommand({
					from_node: from_node?.id,
					from_pin: sourceHandle,
					to_node: to_node?.id,
					to_pin: targetHandle,
				});
				await executeCommand(command);
				setEdges((eds) => eds.filter((e) => e.id !== edge.id));
			}

			edgeReconnectSuccessful.current = true;
		},
		[setEdges, pinToNode, version, executeCommand, board.data?.nodes],
	);

	const onContextMenuCB = useCallback((event: any) => {
		setClickPosition({ x: event.clientX, y: event.clientY });
	}, []);

	const onNodeDragStop = useCallback(
		async (event: any, node: any, nodes: any) => {
			// Don't execute commands when viewing an old version
			if (typeof version !== "undefined") {
				return;
			}
			const commands: IGenericCommand[] = [];
			for await (const node of nodes) {
				const command = moveNodeCommand({
					node_id: node.id,
					to_coordinates: [node.position.x, node.position.y, 0],
					current_layer: currentLayer,
				});

				commands.push(command);
			}
			await executeCommands(commands);
		},
		[boardId, executeCommands, currentLayer, version],
	);

	const isValidConnectionCB = useCallback(
		(connection: Edge | Connection) => {
			return isValidConnection(connection, pinCache, board.data?.refs ?? {});
		},
		[pinCache, board.data?.refs],
	) as IsValidConnection<Edge>;

	const onNodeDoubleClick = useCallback(
		(event: any, node: any) => {
			const tgt = event.target as HTMLElement;
			if (tgt.closest("input, textarea")) {
				return;
			}
			const type = node?.type ?? "";
			if (type === "layerNode") {
				const layer: ILayer = node.data.layer;
				pushLayer(layer);
				return;
			}
			if (type === "callFunctionNode") {
				const layerId = node?.data?.functionLayerId as string | undefined;
				if (layerId && board.data?.layers?.[layerId]) {
					pushLayer(board.data.layers[layerId]);
				}
				return;
			}
		},
		[pushLayer, board.data?.layers],
	);

	const onCommentPlace = useCallback(async () => {
		// Don't execute commands when viewing an old version
		if (typeof version !== "undefined") {
			return;
		}

		const location = screenToFlowPosition({
			x: clickPosition.x,
			y: clickPosition.y,
		});
		const new_comment: IComment = {
			comment_type: ICommentType.Text,
			content: "",
			coordinates: [location.x, location.y, 0],
			id: createId(),
			timestamp: {
				nanos_since_epoch: 0,
				secs_since_epoch: 0,
			},
			author: "anonymous",
		};

		const command = upsertCommentCommand({
			comment: new_comment,
			current_layer: currentLayer,
		});

		await executeCommand(command);
	}, [currentLayer, clickPosition, executeCommand, version]);

	const onNodeDrag = useCallback(
		(event: any, node: Node, nodes: Node[]) => {
			if (shiftPressed) {
				nodes.forEach((node) => {
					if (node.type === "layerNode") {
						const layerData = node.data.layer as ILayer;
						const diffX = Math.abs(node.position.x - layerData.coordinates[0]);
						const diffY = Math.abs(node.position.y - layerData.coordinates[1]);
						if (diffX > diffY) {
							node.position.y = layerData.coordinates[1];
							return;
						}
						node.position.x = layerData.coordinates[0];
						return;
					}

					if (node.type === "commentNode") {
						const commentData = node.data.comment as IComment;
						const diffX = Math.abs(
							node.position.x - commentData.coordinates[0],
						);
						const diffY = Math.abs(
							node.position.y - commentData.coordinates[1],
						);
						if (diffX > diffY) {
							node.position.y = commentData.coordinates[1];
							return;
						}
						node.position.x = commentData.coordinates[0];
						return;
					}

					if (node.type === "node") {
						const nodeData = node.data.node as INode;
						if (!nodeData.coordinates) return;
						const diffX = Math.abs(node.position.x - nodeData.coordinates[0]);
						const diffY = Math.abs(node.position.y - nodeData.coordinates[1]);
						if (diffX > diffY) {
							node.position.y = nodeData.coordinates[1];
							return;
						}
						node.position.x = nodeData.coordinates[0];
					}
				});
			}
		},
		[shiftPressed],
	);

	const onAcceptSuggestion = useCallback(
		async (suggestion: any) => {
			const node = catalog.data?.find((n) => n.name === suggestion.node_type);
			if (node) {
				await placeNode(node);
			} else {
				toastError(`Node type ${suggestion.node_type} not found`, <XIcon />);
			}
		},
		[catalog.data, placeNode],
	);

	const [autoLayoutDialogOpen, setAutoLayoutDialogOpen] = useState(false);

	const autoLayout = useCallback(
		async (style: LayoutStyle = "compact") => {
			if (typeof version !== "undefined") {
				toastError("Cannot modify old version", <XIcon />);
				return;
			}
			const boardData = board.data;
			if (!boardData) return;

			const layerNodes: INode[] = [];
			for (const node of Object.values(boardData.nodes)) {
				const nodeLayer = (node.layer ?? "") === "" ? undefined : node.layer;
				if (nodeLayer === currentLayer) {
					layerNodes.push(node);
				}
			}

			const layerEntities: { id: string; coordinates: number[] }[] = [];
			if (boardData.layers) {
				for (const layer of Object.values(boardData.layers)) {
					if (layer.type === "Function" && layer.id !== currentLayer) continue;
					const parentLayer =
						(layer.parent_id ?? "") === "" ? undefined : layer.parent_id;
					if (parentLayer === currentLayer && layer.id !== currentLayer) {
						layerEntities.push({
							id: layer.id,
							coordinates: [...layer.coordinates],
						});
					}
				}
			}

			if (layerNodes.length === 0 && layerEntities.length === 0) return;

			const newPositions = computeFlowLayout(
				{
					layerNodes,
					layerEntities,
					boardLayers: boardData.layers,
					currentLayer,
				},
				style,
			);

			const commands: IGenericCommand[] = [];
			for (const node of layerNodes) {
				const pos = newPositions.get(node.id);
				if (!pos) continue;
				commands.push(
					moveNodeCommand({
						node_id: node.id,
						from_coordinates: node.coordinates ?? [0, 0, 0],
						to_coordinates: [pos[0], pos[1], 0],
						current_layer: currentLayer,
					}),
				);
			}
			for (const entity of layerEntities) {
				const pos = newPositions.get(entity.id);
				if (!pos) continue;
				commands.push(
					moveNodeCommand({
						node_id: entity.id,
						from_coordinates: entity.coordinates,
						to_coordinates: [pos[0], pos[1], 0],
						current_layer: currentLayer,
					}),
				);
			}

			if (commands.length === 0) return;
			await executeCommands(commands);

			setTimeout(() => fitView({ padding: 0.2, duration: 300 }), 100);
		},
		[board.data, currentLayer, executeCommands, fitView, version],
	);

	// Use the copilot commands hook for executing AI-generated commands
	const { handleExecuteCommands } = useCopilotCommands({
		board,
		catalog,
		executeCommands,
		currentLayer,
	});
	const handleApplyFlowScript = useCallback(
		(
			flowscript: string,
			options?: { allowDeletions?: boolean; suppressBlockedToast?: boolean },
		) => applyFlowScript(flowscript, currentLayer, catalog.data, options),
		[applyFlowScript, currentLayer, catalog.data],
	);
	const handleApplyFlowIrCommit = useCallback(
		(token: Parameters<typeof applyFlowIrCommit>[0]) =>
			applyFlowIrCommit(token),
		[applyFlowIrCommit],
	);

	// Publish the live board surface for the global assistant while this board is
	// mounted (old versions are read-only, so they never register).
	useEffect(() => {
		if (typeof version !== "undefined") return;
		const surface: AssistantBoardSurface = {
			appId,
			boardId,
			board: board.data,
			currentLayer,
			catalogNodes: catalog.data,
			selectedNodeIds,
			runContext: currentMetadata,
			applyFlowScript: handleApplyFlowScript,
			applyFlowIrCommit: handleApplyFlowIrCommit,
			executeCommands: handleExecuteCommands,
			focusNode,
			selectNodes,
			clearRunContext: handleClearRunContext,
		};
		useAssistantSurface.getState().setBoardSurface(surface);
		return () => {
			const store = useAssistantSurface.getState();
			if (store.boardSurface === surface) store.setBoardSurface(null);
		};
	}, [
		version,
		appId,
		boardId,
		board.data,
		currentLayer,
		catalog.data,
		selectedNodeIds,
		currentMetadata,
		handleApplyFlowScript,
		handleApplyFlowIrCommit,
		handleExecuteCommands,
		focusNode,
		selectNodes,
		handleClearRunContext,
	]);

	return (
		<div className="w-full flex flex-1 grow flex-col min-h-0 relative overflow-hidden">
			{/* Desktop FlowPilot floating panel (embedded hosts only) */}
			{!externalAssistant && copilotOpen && (
				<div className="hidden md:block fixed inset-0 z-100 pointer-events-none">
					<div
						className="absolute inset-y-0 right-0 pointer-events-auto transition-[width] duration-300 ease-out"
						style={{
							width: copilotWorkspaceVisible
								? "min(1120px, calc(100vw - 1rem))"
								: "min(500px, calc(100vw - 1rem))",
						}}
					>
						<FlowCopilot
							appId={appId}
							board={board.data}
							catalogNodes={catalog.data}
							selectedNodeIds={selectedNodeIds}
							onAcceptSuggestion={onAcceptSuggestion}
							onFocusNode={focusNode}
							onSelectNodes={selectNodes}
							onExecuteCommands={handleExecuteCommands}
							onApplyFlowScript={handleApplyFlowScript}
							onApplyFlowIrCommit={handleApplyFlowIrCommit}
							runContext={currentMetadata}
							onClearRunContext={handleClearRunContext}
							onClose={handleCopilotClose}
							onWorkspaceVisibleChange={setCopilotWorkspaceVisible}
							mode="panel"
							initialPrompt={copilotInitialPrompt}
						/>
					</div>
				</div>
			)}
			{/* Top-right toolbar - Figma style with connection status */}
			<div className="fixed right-3 top-16 z-50 flex items-center gap-2 sm:right-4 sm:top-16 md:right-6 md:top-6">
				{/* Connection status indicator */}
				{awareness && connectionStatus === "connected" && (
					<div className="flex items-center gap-2 rounded-xl border border-[color-mix(in_oklch,var(--primary)_35%,transparent)] bg-[color-mix(in_oklch,var(--background)_92%,transparent)] px-3 py-1.5 shadow-sm">
						<WifiIcon className="h-3.5 w-3.5 text-primary animate-pulse" />
						<span className="text-xs font-medium text-primary">Live</span>
					</div>
				)}
				{awareness && connectionStatus === "reconnecting" && (
					<div className="flex items-center gap-2 rounded-xl border border-[color-mix(in_oklch,var(--yellow-500)_35%,transparent)] bg-[color-mix(in_oklch,var(--background)_92%,transparent)] px-3 py-1.5 shadow-sm">
						<WifiIcon className="h-3.5 w-3.5 text-yellow-500 animate-pulse" />
						<span className="text-xs font-medium text-yellow-500">
							Reconnecting...
						</span>
					</div>
				)}
				{awareness && connectionStatus === "disconnected" && (
					<button
						type="button"
						onClick={() => reconnect()}
						className="flex items-center gap-2 rounded-xl border border-[color-mix(in_oklch,var(--destructive)_35%,transparent)] bg-[color-mix(in_oklch,var(--background)_92%,transparent)] px-3 py-1.5 shadow-sm hover:bg-[color-mix(in_oklch,var(--background)_85%,transparent)] transition-colors cursor-pointer"
					>
						<WifiOffIcon className="h-3.5 w-3.5 text-destructive" />
						<span className="text-xs font-medium text-destructive">
							Disconnected - Click to reconnect
						</span>
					</button>
				)}
				{!awareness && (
					<div className="flex items-center gap-2 rounded-xl border border-[color-mix(in_oklch,var(--muted-foreground)_35%,transparent)] bg-[color-mix(in_oklch,var(--background)_92%,transparent)] px-3 py-1.5 shadow-sm">
						<WifiOffIcon className="h-3.5 w-3.5 text-muted-foreground" />
						<span className="text-xs font-medium text-muted-foreground">
							Offline
						</span>
					</div>
				)}
				{/* Presence bar with follow mode and chat */}
				{awareness && peerStates.length > 0 && (
					<FlowPresenceBar
						peers={peerStates}
						peerUsers={peerUsers}
						followingSub={followingSub}
						currentLayerPath={layerPath ?? "root"}
						layerNames={layerNames}
						onToggleFollow={toggleFollow}
						onJumpToUser={jumpToUser}
						onJumpToLayer={jumpToLayer}
						onOpenChat={handleToggleChat}
						unreadCount={unreadCount}
					/>
				)}
				{/* Follow mode indicator */}
				{followingSub && (
					<button
						type="button"
						onClick={() => stopFollowing()}
						className="flex items-center gap-2 rounded-xl border border-blue-400/50 bg-blue-500/10 px-3 py-1.5 backdrop-blur-sm shadow-sm hover:bg-blue-500/20 transition-colors cursor-pointer"
					>
						<Eye className="h-3.5 w-3.5 text-blue-400" />
						<span className="text-xs font-medium text-blue-400">
							Following — click or press Esc to stop
						</span>
					</button>
				)}
				{/* Board activity indicator */}
				<BoardActivityIndicator boardId={boardId} />
			</div>
			{/* Floating chat panel */}
			{chatOpen && awareness && (
				<div className="fixed right-3 top-28 z-50 sm:right-4 md:right-6 md:top-20">
					<FlowChat
						messages={chatMessages}
						onSendMessage={sendMessage}
						onClose={() => setChatOpen(false)}
						peerUsers={peerUsers}
						sub={sub}
					/>
				</div>
			)}
			<div className="flex items-center justify-center absolute translate-x-[-50%] mt-5 left-[50dvw] z-40">
				{board.data && editBoard && (
					<BoardMeta
						appId={appId}
						board={board.data}
						boardId={boardId}
						closeMeta={() => setEditBoard(false)}
						version={version}
						selectVersion={(version) => setVersion(version)}
						onPageClick={(pageId) => {
							setEditBoard(false);
							router.push(
								`/page-builder?id=${pageId}&app=${appId}&board=${boardId}`,
							);
						}}
						isOffline={app.data?.visibility === IAppVisibility.Offline}
					/>
				)}
				<FlowDock
					mobileClassName="hidden"
					items={[
						...(typeof parentRegister.boardParents[boardId] === "string" &&
						!currentLayer
							? [
									{
										icon: <ArrowBigLeftDashIcon />,
										title: "Back",
										onClick: async () => {
											const urlWithQuery = parentRegister.boardParents[boardId];
											router.push(urlWithQuery);
										},
									},
								]
							: []),
						{
							icon: <VariableIcon />,
							title: "Variables",
							onClick: async () => {
								toggleVars();
							},
						},

						{
							icon: <LayoutTemplateIcon />,
							title: "Templates",
							onClick: async () => {
								setTemplateSelectorOpen(true);
							},
						},
						{
							icon: <WaypointsIcon />,
							title: "Auto Layout",
							onClick: async () => {
								setAutoLayoutDialogOpen(true);
							},
						},
						{
							icon: <NotebookPenIcon />,
							title: "Manage Board",
							onClick: async () => {
								setEditBoard(true);
							},
						},
						{
							icon: <FileTextIcon />,
							title: "Pages",
							onClick: async () => {
								togglePages();
							},
						},
						{
							icon: <SearchIcon />,
							title: "Search (⌘F / ⌘⇧F sidebar)",
							onClick: async () => {
								setSearchMode("dialog");
								setSearchOpen(true);
							},
							onContextMenu: async () => {
								setSearchMode("sidebar");
								setSearchOpen((prev) => !prev);
							},
						},
						{
							icon: <FileCode2Icon />,
							title: "FlowScript",
							onClick: async () => {
								toggleFlowScript();
							},
						},
						{
							icon: <HistoryIcon />,
							separator: "left",
							title: "Run History",
							onClick: async () => {
								toggleRunHistory();
							},
						},
						...(currentMetadata
							? [
									{
										icon: <ScrollIcon />,
										title: "Logs",
										onClick: async () => {
											toggleLogs();
										},
									},
								]
							: ([] as any)),
						...(currentLayer
							? [
									{
										icon: <SquareChevronUpIcon />,
										title: "Layer Up",
										separator: "left",
										highlight: true,
										onClick: async () => {
											popLayer();
										},
									},
								]
							: []),
						...(extraDockItems ?? []),
						// The floating FlowPilot bubble replaces this dock item when the host
						// provides the global assistant.
						...(externalAssistant
							? []
							: [
									{
										icon: <SparklesIcon className="text-white" />,
										title: "FlowPilot",
										separator: "left",
										special: true,
										onClick: () => openAssistant(),
									},
								]),
					]}
				/>
				{renderOverlay?.()}
			</div>

			{/* Template Selector Overlay - FigJam style centered overlay */}
			{(templateSelectorOpen || (isBoardEmpty && !currentLayer)) && (
				<FlowTemplateSelector
					onSelectTemplate={handleApplyTemplate}
					onDismiss={() => setTemplateSelectorOpen(false)}
				/>
			)}

			<ResizablePanelGroup
				ref={panelGroupRef}
				direction="horizontal"
				className="flex grow flex-1 min-h-0 h-full overscroll-none"
				style={{
					touchAction: "none",
					overflow: "hidden",
				}}
			>
				{/* Desktop/Tablet side panels */}
				<ResizablePanel
					id="flow-variables"
					order={1}
					className="z-50 bg-background hidden md:block"
					defaultSize={0}
					collapsible={true}
					collapsedSize={0}
					ref={varPanelRef}
				>
					{board.data && (
						<VariablesMenu
							board={board.data}
							executeCommand={executeCommand}
							currentLayerId={currentLayer}
							pushLayer={pushLayer}
							boardRef={boardRef}
						/>
					)}
				</ResizablePanel>
				<ResizableHandle withHandle />
				<ResizablePanel id="flow-main-container" order={2}>
					<ResizablePanelGroup
						direction="vertical"
						className="h-full flex grow"
					>
						<ResizablePanel autoSave="flow-main" ref={flowPanelRef}>
							<FlowContextMenu
								board={board.data}
								droppedPin={droppedPin}
								currentLayerId={currentLayer}
								onCommentPlace={onCommentPlace}
								refs={board.data?.refs || {}}
								onClose={() => setDroppedPin(undefined)}
								nodes={catalog.data ?? []}
								onPlaceholder={async (name) => {
									await placePlaceholder(name);
									setDroppedPin(undefined);
								}}
								onNodePlace={async (node) => {
									await placeNode(node);
								}}
								onCreateVariable={async (variable) => {
									const command = upsertVariableCommand({ variable });
									await executeCommand(command, false);
									setDroppedPin(undefined);
								}}
							>
								<div
									className={`w-full h-full relative select-none touch-none ${isOver && "border-green-400 border-2 z-10"}`}
									ref={setNodeRef}
									style={{
										WebkitUserSelect: "none",
										WebkitTouchCallout: "none",
										touchAction: "none",
									}}
									onTouchStart={(e) => {
										const t = e.touches[0];
										if (!t) return;
										const target = e.currentTarget;
										const startX = t.clientX;
										const startY = t.clientY;
										let moved = false;
										const onMove = (me: TouchEvent) => {
											const tt = me.touches[0];
											if (!tt) return;
											if (
												Math.hypot(tt.clientX - startX, tt.clientY - startY) >
												10
											)
												moved = true;
										};
										const timer = setTimeout(() => {
											if (moved) return;
											// Synthesize a contextmenu-like event for long-press
											const evt = new MouseEvent("contextmenu", {
												clientX: startX,
												clientY: startY,
												bubbles: true,
												cancelable: true,
											});
											target.dispatchEvent(evt);
										}, 450);
										const onEnd = () => {
											clearTimeout(timer);
											document.removeEventListener("touchmove", onMove, {
												capture: true,
											} as any);
											document.removeEventListener("touchend", onEnd, {
												capture: true,
											} as any);
											document.removeEventListener("touchcancel", onEnd, {
												capture: true,
											} as any);
										};
										document.addEventListener("touchmove", onMove, {
											passive: true,
											capture: true,
										} as any);
										document.addEventListener("touchend", onEnd, {
											passive: true,
											capture: true,
										} as any);
										document.addEventListener("touchcancel", onEnd, {
											passive: true,
											capture: true,
										} as any);
									}}
								>
									{currentLayer && (
										<h2 className="absolute bottom-0 left-0 z-10 ml-16 mb-10 text-muted pointer-events-none select-none">
											{board.data?.layers[currentLayer]?.name}
										</h2>
									)}
									{version && (
										<h3 className="absolute top-0 mr-2 mt-2 right-0 z-10 text-muted pointer-events-none select-none">
											Version {version[0]}.{version[1]}.{version[2]} - Read-Only
										</h3>
									)}
									<FlowCanvas
										flowRef={flowRef}
										nodes={nodes}
										edges={edges}
										nodeTypes={nodeTypes}
										edgeTypes={edgeTypes}
										colorMode={colorMode}
										nodesInteractive={typeof version === "undefined"}
										onlyRenderVisible={nodes.length > 65}
										currentLayer={currentLayer}
										onContextMenu={onContextMenuCB}
										onInit={initializeFlow}
										onNodeDoubleClick={onNodeDoubleClick}
										onNodesChange={onNodesChangeIntercept}
										onEdgesChange={onEdgesChange}
										onNodeDragStop={onNodeDragStop}
										onNodeDrag={onNodeDrag}
										isValidConnection={isValidConnectionCB}
										onConnect={onConnect}
										onSelectionChange={onSelectionChange}
										onReconnect={onReconnect}
										onReconnectStart={onReconnectStart}
										onMoveEnd={onMoveEnd}
										onReconnectEnd={onReconnectEnd}
										onConnectEnd={onConnectEnd}
										onScreenshot={onScreenshot}
										miniMapNodeColor={miniMapNodeColor}
									/>
									<FlowCursorsLayer
										store={cursorStore}
										currentLayerPath={layerPath ?? "root"}
										peerUsers={peerUsers}
									/>
									{peerStates.length > 0 && (
										<FlowLayerIndicators
											peers={peerStates}
											currentLayerPath={layerPath ?? "root"}
											nodes={nodes}
											peerUsers={peerUsers}
											onJumpToLayer={jumpToLayer}
										/>
									)}
									<DragOverlay
										dropAnimation={{
											duration: 500,
											easing: "cubic-bezier(0.18, 0.67, 0.6, 1.22)",
										}}
									>
										{active?.data?.current?.type === "function-layer" ? (
											<div className="flex items-center gap-2 rounded-md bg-background border px-3 py-2 shadow-md">
												<SquareFunctionIcon className="w-4 h-4 text-violet-500" />
												<span className="text-sm font-medium">
													{board.data?.layers?.[active.data.current.layerId]
														?.name ?? "Function"}
												</span>
											</div>
										) : (active?.data?.current as IVariable)?.id ? (
											<Variable
												variable={active?.data?.current as IVariable}
												preview
												onVariableChange={() => {}}
												onVariableDeleted={() => {}}
											/>
										) : null}
									</DragOverlay>
								</div>
							</FlowContextMenu>
						</ResizablePanel>
						<ResizableHandle withHandle />
						<ResizablePanel
							className="z-50 hidden md:block"
							hidden={!currentMetadata}
							ref={logPanelRef}
							defaultSize={0}
							collapsedSize={0}
							collapsible={true}
							autoSave="flow-logs"
						>
							{board.data && currentMetadata && (
								<Traces
									appId={appId}
									boardId={boardId}
									board={boardRef}
									onFocusNode={focusNode}
									nodeIdFilter={logNodeIdFilter}
									onClearNodeIdFilter={() => setLogNodeIdFilter(undefined)}
								/>
							)}
						</ResizablePanel>
					</ResizablePanelGroup>
				</ResizablePanel>
				<ResizableHandle withHandle />
				<ResizablePanel
					id="flow-runs"
					order={3}
					className="z-50 hidden md:block"
					defaultSize={0}
					collapsible={true}
					collapsedSize={0}
					ref={runsPanelRef}
				>
					{board.data && (
						<FlowRuns
							executeBoard={executeBoard}
							nodes={board.data.nodes}
							appId={appId}
							boardId={boardId}
							version={board.data.version as [number, number, number]}
							onVersionChange={setVersion}
							onFocusNode={focusNode}
						/>
					)}
				</ResizablePanel>
				<ResizableHandle withHandle />
				<ResizablePanel
					id="flow-flowscript"
					order={4}
					className="z-50 hidden md:block"
					defaultSize={0}
					collapsible={true}
					collapsedSize={0}
					ref={flowScriptPanelRef}
					onExpand={() => setFlowScriptPanelVisible(true)}
					onCollapse={() => setFlowScriptPanelVisible(false)}
				>
					{board.data && flowScriptPanelVisible && (
						<FlowScriptPanel
							appId={appId}
							boardId={boardId}
							version={version}
							boardUpdatedAt={board.dataUpdatedAt}
							catalogNodes={catalog.data}
							onApplyFlowScript={handleApplyFlowScript}
							onClose={() => setSidePanelSize(FLOWSCRIPT_PANEL_INDEX, 0)}
						/>
					)}
				</ResizablePanel>
				{searchMode === "sidebar" && searchOpen && (
					<>
						<ResizableHandle withHandle />
						<ResizablePanel
							id="flow-search"
							order={5}
							className="z-50 hidden md:block min-w-[280px] max-w-[400px]"
							defaultSize={20}
							minSize={15}
							maxSize={30}
						>
							<FlowSearch
								board={board.data}
								open={searchOpen}
								onOpenChange={setSearchOpen}
								onNavigate={focusNode}
								mode="sidebar"
							/>
						</ResizablePanel>
					</>
				)}
				{/* Mobile sheets */}
				<Sheet open={varsOpen} onOpenChange={setVarsOpen}>
					<SheetContent side="bottom" className="h-[60dvh] w-full">
						<SheetHeader>
							<SheetTitle>Variables & Functions</SheetTitle>
						</SheetHeader>
						{board.data && (
							<div className="h-[calc(60dvh-3.5rem)] overflow-y-auto overscroll-contain">
								<VariablesMenu
									board={board.data}
									executeCommand={executeCommand}
									currentLayerId={currentLayer}
									pushLayer={pushLayer}
									boardRef={boardRef}
								/>
							</div>
						)}
					</SheetContent>
				</Sheet>
				<Sheet open={runsOpen} onOpenChange={setRunsOpen}>
					<SheetContent side="bottom" className="h-[80dvh] w-full">
						<SheetHeader>
							<SheetTitle>Runs</SheetTitle>
						</SheetHeader>
						{board.data && (
							<div className="h-[calc(80dvh-3.5rem)] overflow-y-auto overscroll-contain">
								<FlowRuns
									executeBoard={executeBoard}
									nodes={board.data.nodes}
									appId={appId}
									boardId={boardId}
									version={board.data.version as [number, number, number]}
									onVersionChange={setVersion}
									onFocusNode={focusNode}
								/>
							</div>
						)}
					</SheetContent>
				</Sheet>
				<Sheet open={logsOpen} onOpenChange={setLogsOpen}>
					<SheetContent side="bottom" className="h-[80dvh] w-full">
						<SheetHeader>
							<SheetTitle>Logs</SheetTitle>
						</SheetHeader>
						{board.data && currentMetadata && (
							<div className="h-[calc(80dvh-3.5rem)] w-full">
								<Traces
									appId={appId}
									boardId={boardId}
									board={boardRef}
									onFocusNode={focusNode}
									nodeIdFilter={logNodeIdFilter}
									onClearNodeIdFilter={() => setLogNodeIdFilter(undefined)}
								/>
							</div>
						)}
						{(!currentMetadata || !board.data) && (
							<div className="h-[calc(80dvh-3.5rem)] w-full flex items-center justify-center text-sm text-muted-foreground p-6">
								No run selected yet. Start a run to view logs here.
							</div>
						)}
					</SheetContent>
				</Sheet>
				{/* FlowScript Sheet (mobile) */}
				<Sheet open={flowScriptSheetOpen} onOpenChange={setFlowScriptSheetOpen}>
					<SheetContent side="bottom" className="h-[90dvh] w-full p-0">
						<SheetHeader className="px-4 pt-4">
							<SheetTitle>FlowScript</SheetTitle>
						</SheetHeader>
						{board.data && flowScriptSheetOpen && (
							<div className="h-[calc(90dvh-3.5rem)] w-full">
								<FlowScriptPanel
									appId={appId}
									boardId={boardId}
									version={version}
									boardUpdatedAt={board.dataUpdatedAt}
									catalogNodes={catalog.data}
									onApplyFlowScript={handleApplyFlowScript}
									onClose={() => setFlowScriptSheetOpen(false)}
								/>
							</div>
						)}
					</SheetContent>
				</Sheet>
				{/* Pages Sheet */}
				<Sheet open={pagesOpen} onOpenChange={setPagesOpen}>
					<SheetContent side="right" className="w-[400px] sm:w-[540px] p-0">
						<FlowPages
							appId={appId}
							boardId={boardId}
							onOpenPage={(pageId, bId) => {
								setPagesOpen(false);
								router.push(
									`/page-builder?id=${pageId}&app=${appId}&board=${bId}`,
								);
							}}
						/>
					</SheetContent>
				</Sheet>
				{/* Mobile FlowPilot Sheet (embedded hosts only) */}
				<Sheet
					open={!externalAssistant && copilotOpen && isMobile}
					onOpenChange={(open) => {
						setCopilotOpen(open);
						if (!open) setCopilotInitialPrompt(undefined);
					}}
				>
					<SheetContent side="bottom" className="h-[100dvh] w-full p-0">
						<div className="h-full w-full">
							<FlowCopilot
								appId={appId}
								board={board.data}
								catalogNodes={catalog.data}
								selectedNodeIds={selectedNodeIds}
								onAcceptSuggestion={onAcceptSuggestion}
								onFocusNode={focusNode}
								onSelectNodes={selectNodes}
								onExecuteCommands={handleExecuteCommands}
								onApplyFlowScript={handleApplyFlowScript}
								onApplyFlowIrCommit={handleApplyFlowIrCommit}
								runContext={currentMetadata}
								onClearRunContext={handleClearRunContext}
								onClose={handleCopilotClose}
								onWorkspaceVisibleChange={setCopilotWorkspaceVisible}
								mode="panel"
								initialPrompt={copilotInitialPrompt}
							/>
						</div>
					</SheetContent>
				</Sheet>
			</ResizablePanelGroup>
			<PinEditModal appId={appId} boardId={boardId} version={version} />
			<FlowNodeInfoOverlay
				key={boardId}
				ref={nodeInfoOverlayRef}
				refs={board.data?.refs}
				boardRef={boardRef}
				onFocusNode={focusNode}
			/>
			{searchMode === "dialog" && (
				<FlowSearch
					board={board.data}
					open={searchOpen}
					onOpenChange={setSearchOpen}
					onNavigate={focusNode}
					mode="dialog"
					onSwitchToSidebar={() => {
						setSearchOpen(false);
						setSearchMode("sidebar");
						// Use setTimeout to ensure state updates properly
						setTimeout(() => setSearchOpen(true), 0);
					}}
				/>
			)}

			{/* Runtime Variables Prompt */}
			<RuntimeVariablesPrompt
				open={runtimeVarsPromptOpen}
				onOpenChange={setRuntimeVarsPromptOpen}
				variables={runtimeConfiguredVars}
				existingValues={existingRuntimeVars}
				onSave={handleRuntimeVarsSave}
				onCancel={handleRuntimeVarsCancel}
			/>

			{/* WASM Sandbox Warning */}
			<WasmSandboxWarningDialog
				open={wasmDialogOpen}
				packageIds={wasmPackageIds}
				packagePermissions={wasmPackagePermissions}
				onConfirm={handleWasmConfirm}
				onCancel={handleWasmCancel}
			/>

			{/* Auto Layout Algorithm Picker */}
			<AutoLayoutDialog
				open={autoLayoutDialogOpen}
				onOpenChange={setAutoLayoutDialogOpen}
				onSelect={(alg) => autoLayout(alg)}
			/>
		</div>
	);
}
