"use client";
import { DragOverlay, useDroppable } from "@dnd-kit/core";
import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { type UseQueryResult, useQueryClient } from "@tanstack/react-query";
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
	Columns2Icon,
	Eye,
	FileCode2Icon,
	FilesIcon,
	FlaskConicalIcon,
	GitBranchIcon,
	HistoryIcon,
	HouseIcon,
	LayoutTemplateIcon,
	MessageSquareIcon,
	NotebookPenIcon,
	PanelBottomIcon,
	PencilLineIcon,
	PlayCircleIcon,
	ScrollIcon,
	SearchIcon,
	ShareIcon,
	SlidersHorizontalIcon,
	SparklesIcon,
	SquareChevronUpIcon,
	SquareFunctionIcon,
	TagIcon,
	TriangleAlertIcon,
	VariableIcon,
	WaypointsIcon,
	WifiIcon,
	WifiOffIcon,
	XIcon,
	ZapIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import { usePathname, useRouter } from "next/navigation";
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
import {
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	useHub,
	useLogAggregation,
	useMobileHeader,
} from "../..";
import { BoardActivityIndicator } from "../../components/flow/board-activity-indicator";
import {
	BoardSyncRecoveryDialog,
	BoardSyncStatusPill,
	useBoardSyncRecoveryRequests,
} from "../../components/flow/board-sync-recovery";
import { CommentNode } from "../../components/flow/comment-node";
import { FlowContextMenu } from "../../components/flow/flow-context-menu";
import { FlowNode } from "../../components/flow/flow-node";
import { EventPayloadForm } from "../../components/flow/flow-node/event-payload-form";
import {
	FlowNodeInfoOverlay,
	type FlowNodeInfoOverlayHandle,
} from "../../components/flow/flow-node/flow-node-info-overlay";
import { deriveRunCapabilities } from "../../components/flow/flow-run-capabilities";
import { catalogNamespaceRoots } from "../../components/flow/flowscript/flowscript-language";
import { FlowScriptPanel } from "../../components/flow/flowscript/flowscript-panel";
import { resolveJoinableScopeNodeIds } from "../../components/flow/flowscript/flowscript-panel-state";
import {
	collectCommandEntityIds,
	findClaimCollision,
	readPeerFlowScriptClaims,
	useFlowScriptCanvasPresence,
	useFlowScriptPeerScopes,
} from "../../components/flow/flowscript/flowscript-presence";
import type {
	FlowScriptRunCapability,
	FlowScriptRunMode,
} from "../../components/flow/flowscript/flowscript-run-lens";
import { useFlowScriptFiles } from "../../components/flow/flowscript/use-flowscript-files";
import { MediaNode } from "../../components/flow/media-node";
import { BoardAccountItem } from "../../components/flow/shell/board-account-item";
import { BoardActivityRail } from "../../components/flow/shell/board-activity-rail";
import type { IBoardRailItem } from "../../components/flow/shell/board-activity-rail";
import { BoardBreadcrumb } from "../../components/flow/shell/board-breadcrumb";
import { BoardEditorActions } from "../../components/flow/shell/board-editor-actions";
import type { IBoardEditorAction } from "../../components/flow/shell/board-editor-actions";
import { BoardExplorer } from "../../components/flow/shell/board-explorer";
import { BoardInspector } from "../../components/flow/shell/board-inspector";
import {
	BoardIdentityForm,
	BoardReleaseForm,
	BoardRuntimeForm,
	executionModeIcon,
} from "../../components/flow/shell/board-meta-controls";
import { BoardMobileHost } from "../../components/flow/shell/board-mobile-host";
import { BoardNavMenu } from "../../components/flow/shell/board-nav-menu";
import { BoardPane, BoardPanel } from "../../components/flow/shell/board-panes";
import { BoardShell } from "../../components/flow/shell/board-shell";
import {
	BoardStatusBar,
	BoardStatusItem,
} from "../../components/flow/shell/board-status-bar";
import {
	fileAfterClose,
	withFileClosed,
	withFileOpen,
	withMissingFilesDropped,
} from "../../components/flow/shell/open-files";
import type { IBoardCommand } from "../../components/flow/shell/use-board-commands";
import {
	commandsFor,
	formatShortcut,
	useBoardCommands,
} from "../../components/flow/shell/use-board-commands";
import { useBoardSurface } from "../../components/flow/shell/use-board-surface";
import { Traces } from "../../components/flow/traces";
import { UploadPlaceholderNode } from "../../components/flow/upload-placeholder-node";
import { typeToColor } from "../../components/flow/utils";
import { VariablesMenu } from "../../components/flow/variables/variables-menu";
import { useCommandExecution } from "../../hooks/use-command-execution";
import { useCopilotCommands } from "../../hooks/use-copilot-commands";
import { useExecutionPresence } from "../../hooks/use-execution-presence";
import {
	type FollowedEditorAnchor,
	useFollowMode,
} from "../../hooks/use-follow-mode";
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
	discoverBoardTests,
	moveNodeCommand,
	moveToLayerCommand,
	removeCommentCommand,
	removeLayerCommand,
	removeNodeCommand,
	updateNodeCommand,
	upsertCommentCommand,
	upsertVariableCommand,
} from "../../lib";
import { ownsWindowChrome } from "../../lib/chrome-route";
import { getErrorMessage } from "../../lib/error-message";
import {
	type LayoutBox,
	type LayoutComment,
	computeFlowLayoutDetailed,
} from "../../lib/flow-auto-layout";
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
import {
	FLOWSCRIPT_KEYWORDS,
	MAIN_FILE_LABEL,
	MODULE_FILE_EXTENSION,
	activeModuleId,
	boardFlowScriptScope,
	boardModules,
	fileModuleId,
	moduleFileId,
	modulePathLabel,
} from "../../lib/flow-modules";
import { onFlowScriptNamesTableLoaded } from "../../lib/flowscript/names";
import { toastError, toastSuccess, toastWarning } from "../../lib/messages";
import { plainTextFromRichContent } from "../../lib/plate-text";
import { isWebkitLite } from "../../lib/platform";
import { getRuntimeConfiguredVariables } from "../../lib/runtime-vars-utils";
import { IAppVisibility } from "../../lib/schema/app/app";
import type { IBit } from "../../lib/schema/bit/bit";
import { IExecutionMode } from "../../lib/schema/flow/board";
import {
	type IBoard,
	type IComment,
	ICommentType,
	ILayerType,
	type IVariable,
} from "../../lib/schema/flow/board";
import { type INode, IVariableType } from "../../lib/schema/flow/node";
import type { IPin } from "../../lib/schema/flow/pin";
import type { ILayer } from "../../lib/schema/flow/run";
import { buildTemplateCopyPasteCommand } from "../../lib/template-copy-paste";
import { convertJsonToUint8Array } from "../../lib/uint8";
import {
	type AssistantBoardSurface,
	useAssistantSurface,
} from "../../state/assistant-surface";
import { useBackend } from "../../state/backend-state";
import {
	boardTestSummary,
	useBoardTestsStore,
} from "../../state/board-tests-state";
import { useRequestFabBubble } from "../../state/fab-bubble";
import { useFlowBoardParentState } from "../../state/flow-board-parent-state";
import { useRunExecutionStore } from "../../state/run-execution-state";
import {
	type RuntimeVariableValue,
	useRuntimeVariables,
} from "../../state/runtime-variables-context";
import { AutoLayoutDialog, type LayoutStyle } from "./auto-layout-dialog";
import { CallFunctionNode } from "./call-function-node";
import { FlowChat } from "./flow-chat";
import { FlowCopilot } from "./flow-copilot";
import type { FlowScriptApplyOptions } from "./flow-copilot/types";
import { FlowCursorsLayer } from "./flow-cursors";
import { FlowDataEdge } from "./flow-data-edge";
import { FlowExecutionEdge } from "./flow-execution-edge";
import { useUndoRedo } from "./flow-history";
import { FlowLayerIndicators } from "./flow-layer-indicators";
import { FlowModuleTabs } from "./flow-module-tabs";
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
import { FlowTests } from "./flow-tests";
import { FlowVeilEdge } from "./flow-veil-edge";
import { LayerInnerNode } from "./layer-inner-node";
import { LayerNode } from "./layer-node";
import { RuntimeVariablesPrompt } from "./runtime-variables-prompt";
import { WasmSandboxWarningDialog } from "./wasm-sandbox-warning-dialog";

const REMOTE_BOARD_APPLIED_EVENT = "flow:remote-board-applied";

/**
 * Canvas node types a "move to module" carries: nodes and comments. A layer keeps its
 * own file through its parent chain, so it is not re-filed from the canvas.
 */
const MOVABLE_SELECTION_TYPES = new Set([
	"node",
	"flowNode",
	"callFunctionNode",
	"commentNode",
	"mediaNode",
]);

/** Same ids, order-insensitive — keeps a re-selection from re-rendering half the board. */
const sameIds = (previous: string[], next: string[]): boolean => {
	if (previous.length !== next.length) return false;
	const known = new Set(previous);
	return next.every((id) => known.has(id));
};

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
	/** Inside a layer with a boundary. A module is a file, not a place, so it reads as root. */
	insideLayer: boolean;
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
	insideLayer,
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
	const { t } = useTranslation("flow");
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
				bgColor={
					isWebkitLite()
						? "var(--background)"
						: "color-mix(in oklch, var(--background) 80%, transparent)"
				}
				maskColor={
					isWebkitLite()
						? "rgb(127 127 127 / 0.15)"
						: "color-mix(in oklch, var(--foreground) 10%, transparent)"
				}
				nodeColor={miniMapNodeColor}
			/>
			<Background
				variant={insideLayer ? BackgroundVariant.Lines : BackgroundVariant.Dots}
				color={
					insideLayer
						? `color-mix(in oklch, var(--foreground) 5%, transparent)`
						: `color-mix(in oklch, var(--foreground) 20%, transparent)`
				}
				bgColor="color-mix(in oklch, var(--background) 80%, transparent)"
				gap={12}
				size={1}
			/>
		</ReactFlow>
	);
});

const PROFILE_BITS_STALE_TIME = 5 * 60 * 1000;

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
	const { t } = useTranslation("flow");
	// Without an in-interface FlowPilot button the floating bubble is this board's only way into the
	// assistant, so ask for it exactly when we drop our own.
	useRequestFabBubble(externalAssistant);
	const {
		pushCommand,
		pushCommands,
		pushCommandsOnce,
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
	// Selector, not the whole store: `boardParents` is one global map, so
	// registering a parent for any board in any app re-rendered the entire board.
	const boardParent = useFlowBoardParentState(
		(state) => state.boardParents[boardId],
	);
	// FlowBoard is also embedded — the university lesson workspace mounts it
	// beside its own reading pane, where the global sidebar is still there and
	// the host owns navigation. Only the route that unmounts that sidebar may
	// grow the board's own way out.
	const ownsWindow = ownsWindowChrome(usePathname());
	// Where "out" goes when nothing registered a parent — the app's flow list,
	// which keeps the app context that "/" throws away. Without an app there is
	// no such list, so fall back to the root.
	const appHref = useMemo(
		() => (appId ? `/library/config/flows?id=${appId}` : "/"),
		[appId],
	);
	// Board-owned navigation exists when the board can actually go somewhere:
	// a registered parent, or the board owning the window and falling back home.
	const canNavigateOut = Boolean(boardParent) || ownsWindow;
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
	const [flowInstanceReady, setFlowInstanceReady] = useState(false);
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
	const queryClient = useQueryClient();
	const selectorDataRef = useRef(createEmptyFlowSelectorData());
	const [selectorDataVersion, setSelectorDataVersion] = useState(0);
	const selectorCacheKeyRef = useRef("");
	const elementOptionsPromiseRef = useRef<
		Promise<FlowElementOption[]> | undefined
	>(undefined);
	const bitOptionsPromiseRef = useRef<Promise<IBit[]> | undefined>(undefined);
	// getProfile() already resolves to the hub profile, so `id` is the reactive
	// source here — `backend.profile` is only assigned later, out of band.
	const selectorProfileId =
		currentProfile.data?.id ?? backend.profile?.id ?? backend.profile?.hub;
	const selectorCacheKey = `${selectorProfileId ?? "local"}:${appId}`;

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
					// Shared query key with useInvoke(getProfileBits): every bit pin on
					// the board resolves from one request, reused across board mounts.
					// The query cache is persisted, so it is only safe once the profile
					// is known — an unresolved id would cache one profile's bits under a
					// key every other profile also reads from.
					const bits = selectorProfileId
						? await queryClient.fetchQuery({
								queryKey: ["getProfileBits", selectorProfileId],
								queryFn: () => backend.bitState.getProfileBits(),
								staleTime: force ? 0 : PROFILE_BITS_STALE_TIME,
							})
						: await backend.bitState.getProfileBits();
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
		[backend.bitState, queryClient, selectorProfileId],
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
	const [currentLayer, setCurrentLayer] = useState<string | undefined>();
	const [layerPath, setLayerPath] = useState<string | undefined>();
	// The file the canvas is in: a module open on screen, or the module owning whatever layer
	// is. Null is main — the board root, which is not a layer.
	const currentModuleId = useMemo(
		() => activeModuleId(layerPath, currentLayer, board.data?.layers),
		[layerPath, currentLayer, board.data?.layers],
	);
	// A module has no boundary and draws no frame, so its canvas is the root's canvas.
	const insideModule =
		Boolean(currentLayer) && currentModuleId === currentLayer;
	const modules = useMemo(
		() => boardModules(board.data?.layers),
		[board.data?.layers],
	);
	// The board half of the FlowScript editor's world: what the modules are called and which
	// functions live in each, so a file that calls into another file is not linted as unknown.
	const flowScriptBoardScope = useMemo(
		() => boardFlowScriptScope(board.data?.layers),
		[board.data?.layers],
	);
	/** The FlowScript file the canvas is on — `main` or the module it is inside. */
	const currentFileId = moduleFileId(currentModuleId);
	const currentModuleIdRef = useRef(currentModuleId);
	currentModuleIdRef.current = currentModuleId;

	// Reaching a module any other way — entering it on canvas, following a peer,
	// a deep link — opens its tab as well, or the strip would disagree with the
	// canvas about what is open.
	useEffect(() => {
		if (!currentModuleId) return;
		setOpenFileIds((old) =>
			old.includes(currentModuleId) ? old : [...old, currentModuleId],
		);
	}, [currentModuleId]);

	// A module that no longer exists cannot keep a tab.
	useEffect(() => {
		const layers = board.data?.layers;
		if (!layers) return;
		setOpenFileIds((old) => {
			const next = withMissingFilesDropped(old, (id) => Boolean(layers[id]));
			return next.length === old.length ? old : next;
		});
	}, [board.data?.layers]);
	// A module named after a catalog namespace root would make every qualified call inside it
	// ambiguous, so the catalog's roots are reserved alongside the language's keywords. The roots
	// are only complete once the FlowScript names snapshot is in — nothing here loads it (the
	// module name field does), this just recomputes when it arrives.
	const [flowScriptNamesReady, setFlowScriptNamesReady] = useState(false);
	useEffect(
		() => onFlowScriptNamesTableLoaded(() => setFlowScriptNamesReady(true)),
		[],
	);
	const moduleReservedRoots = useMemo(
		() => [...FLOWSCRIPT_KEYWORDS, ...catalogNamespaceRoots(catalog.data)],
		// biome-ignore lint/correctness/useExhaustiveDependencies: the names snapshot is a recompute trigger, read inside catalogNamespaceRoots
		[catalog.data, flowScriptNamesReady],
	);
	// One buffer per file, kept here: the panel mounts twice (desktop panel, mobile sheet) and
	// both must hand the same drafts back and forth.
	const flowScriptFiles = useFlowScriptFiles();
	const flowScriptFilesClear = flowScriptFiles.clear;
	// Drafts belong to one board at one version; switching either makes every stashed file stale.
	useEffect(() => {
		flowScriptFilesClear();
	}, [boardId, version, flowScriptFilesClear]);
	const [templateSelectorOpen, setTemplateSelectorOpen] = useState(false);
	const [runtimeVarsPromptOpen, setRuntimeVarsPromptOpen] = useState(false);
	const [pendingExecution, setPendingExecution] = useState<{
		node: INode;
		payload?: object;
		isRemote: boolean;
		/** When set, the prompt hands the saved variables back instead of executing `node`. */
		resume?: (runtimeVariables?: Record<string, IVariable>) => void;
		cancel?: () => void;
	} | null>(null);
	const deleteSelectionInFlightRef = useRef(false);
	const [existingRuntimeVars, setExistingRuntimeVars] = useState<
		Map<string, RuntimeVariableValue>
	>(new Map());
	const runtimeVarsContext = useRuntimeVariables();
	const boardTestEntries = useBoardTestsStore(
		(state) => state.entries[boardId],
	);
	const boardTestNodeIds = useMemo(
		() =>
			new Set(
				discoverBoardTests(board.data?.nodes).map((test) => test.node.id),
			),
		[board.data?.nodes],
	);
	const boardTestsFailed = useMemo(
		() => boardTestSummary(boardTestEntries, boardTestNodeIds).failed,
		[boardTestEntries, boardTestNodeIds],
	);
	const colorMode = useMemo(
		() => (resolvedTheme === "dark" ? "dark" : "light"),
		[resolvedTheme],
	);

	const { update: updateHeader } = useMobileHeader();

	useEffect(() => {
		const left: ReactElement[] = [];
		const right: ReactElement[] = [];

		if (canNavigateOut) {
			left.push(
				<Button
					variant={"default"}
					size={"icon"}
					aria-label={
						boardParent ? t("backToApp", "Back to app") : t("home", "Home")
					}
					onClick={() => router.push(boardParent ?? appHref)}
				>
					{boardParent ? <ArrowBigLeftDashIcon /> : <HouseIcon />}
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
				aria-label={t("openLogs", "Open logs")}
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
					aria-label={t("openFlowpilot", "Open FlowPilot")}
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

		// Modules are left through their tab, not by climbing out of them.
		if (currentLayer && !insideModule) {
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
		insideModule,
		boardParent,
		appHref,
		canNavigateOut,
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

	const { saveViewport, holdViewport } = useViewportManager({
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
		holdViewport,
		fitView,
		getNodes,
	});

	// Opening a module file is nothing but making it the current layer, so it inherits the
	// per-layer viewport and the layer trail. The tab is only a no-op when the canvas already
	// shows that exact file — from a layer *inside* a module it walks back out to it.
	const selectModule = useCallback(
		async (moduleId: string | null) => {
			if ((currentLayer ?? null) === moduleId) return;
			if (!moduleId) {
				await saveViewport();
				setCurrentLayer(undefined);
				setLayerPath(undefined);
				return;
			}
			const layer = board.data?.layers?.[moduleId];
			if (layer) await pushLayer(layer);
		},
		[board.data?.layers, currentLayer, pushLayer, saveViewport],
	);

	// Which files have a tab. `main` is the board itself and is always open; every
	// other entry is a module the user opened, so the strip lists what is open
	// while the explorer lists what exists.
	const [openFileIds, setOpenFileIds] = useState<string[]>([]);
	const openFileIdsRef = useRef(openFileIds);
	openFileIdsRef.current = openFileIds;

	const handleSelectModule = useCallback(
		(moduleId: string | null) => {
			if (moduleId) setOpenFileIds((old) => withFileOpen(old, moduleId));
			void selectModule(moduleId);
		},
		[selectModule],
	);

	const handleCloseFile = useCallback(
		(moduleId: string) => {
			// Closing the file on screen moves to its neighbour; `null` is main.flow.
			if (currentModuleIdRef.current === moduleId) {
				void selectModule(fileAfterClose(openFileIdsRef.current, moduleId));
			}
			setOpenFileIds((old) => withFileClosed(old, moduleId));
		},
		[selectModule],
	);

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
		pushCommandsOnce,
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

	// Peers' FlowScript editor cursors/claims projected onto canvas nodes
	// (peer-colored outline + "being edited by" badge).
	useFlowScriptCanvasPresence({ awareness, sub, setNodes });

	// Peers' shared FlowScript scopes (sub → node ids) for the presence bar's
	// "Join code scope" action.
	const peerScopes = useFlowScriptPeerScopes({ awareness, sub });

	// Cross-surface follow: the concrete handler is bound below once the
	// FlowScript panel state exists; the ref keeps this callback stable.
	const followEditorAnchorRef = useRef<(anchor: FollowedEditorAnchor) => void>(
		() => {},
	);
	const handleFollowEditorAnchor = useCallback(
		(anchor: FollowedEditorAnchor) => followEditorAnchorRef.current(anchor),
		[],
	);

	// Follow mode
	const { followingSub, toggleFollow, stopFollowing } = useFollowMode({
		awareness,
		sub,
		setViewport,
		getViewport,
		onFollowEditorAnchor: handleFollowEditorAnchor,
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

			// Navigate to peer's layer if different. Held while the layer swaps so the
			// per-layer viewport restore does not overwrite the peer's viewport below.
			if (peerLayer !== myLayer) {
				const release = holdViewport();
				if (peerLayer === "root" || !peerLayer) {
					setLayerPath(undefined);
					setCurrentLayer(undefined);
				} else {
					setLayerPath(peerLayer);
					const segments = peerLayer.split("/");
					setCurrentLayer(segments[segments.length - 1]);
				}
				setTimeout(release, 600);
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
			holdViewport,
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

	// Undelivered board edits: the only exit when the outbox cannot drain.
	const [syncRecoveryOpen, setSyncRecoveryOpen] = useState(false);
	const openSyncRecovery = useCallback(() => setSyncRecoveryOpen(true), []);
	useBoardSyncRecoveryRequests(appId, boardId, openSyncRecovery);

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
	const { remoteExecutingNodeIds, remoteExecutions } = useExecutionPresence({
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

	const initializeFlow = useCallback(async (_instance: ReactFlowInstance) => {
		setFlowInstanceReady(true);
	}, []);

	// React Flow commonly initializes before the async board query completes. Focusing from
	// `onInit` therefore used to consume the deep-link exactly once while `board.data` was still
	// empty. Wait for both halves instead; this is also the deterministic boundary used by the
	// workflow screenshot CLI's `--focus-node` option.
	useEffect(() => {
		if (initialized || !flowInstanceReady || !board.data) return;
		if (!nodeId || nodeId === "") return;
		focusNode(nodeId);
		setInitialized(true);
	}, [board.data, flowInstanceReady, focusNode, initialized, nodeId]);

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
				toastError(
					t("cannotModifyOldVersion", "Cannot modify old version"),
					<XIcon />,
				);
				return;
			}

			try {
				const templateBoard = await backend.templateState.getTemplate(
					templateAppId,
					templateId,
				);

				if (!templateBoard) {
					toastError(t("templateNotFound", "Template not found"), <XIcon />);
					return;
				}

				const templateNodes = Object.values(templateBoard.nodes);
				const templateComments = Object.values(templateBoard.comments);
				const templateLayers = Object.values(templateBoard.layers);

				if (
					templateNodes.length === 0 &&
					templateComments.length === 0 &&
					templateLayers.length === 0
				) {
					toastError(t("templateIsEmpty", "Template is empty"), <XIcon />);
					return;
				}

				await executeCommand(
					buildTemplateCopyPasteCommand(templateBoard, currentLayer),
				);
				setTemplateSelectorOpen(false);
			} catch (error) {
				console.error("Failed to apply template:", error);
				toastError(
					t("failedToApplyTemplate", "Failed to apply template"),
					<XIcon />,
				);
			}
		},
		[backend.templateState, executeCommand, currentLayer, version],
	);

	const isMobile = useMediaQuery("(max-width: 767px)");
	// One owner for every dockable surface. Replaces six independent booleans and
	// the four imperative panel handles that used to hand a closing panel's width
	// to its neighbour instead of back to the canvas.
	const { surface: shell, actions: surfaceActions } = useBoardSurface(isMobile);
	const flowScriptPanelVisible = shell.script;
	const flowScriptSheetOpen = shell.mobile === "script";
	// Node ids the FlowScript panel is scoped to ("Edit selection as FlowScript").
	const [flowScriptScope, setFlowScriptScope] = useState<string[] | undefined>(
		undefined,
	);
	// Follow mode → editor: bumped when the followed peer's text cursor moves to
	// a new statement; the open panel reveals + flashes that anchor's line.
	const [flowScriptRevealRequest, setFlowScriptRevealRequest] = useState<
		{ nodeId: string; token: number } | undefined
	>(undefined);
	const flowScriptPanelOpenRef = useRef(false);
	flowScriptPanelOpenRef.current =
		flowScriptPanelVisible || flowScriptSheetOpen;
	// The followed peer is typing in THEIR panel: reveal in ours when open,
	// otherwise focus the node on canvas. Never auto-opens the panel.
	followEditorAnchorRef.current = (anchor: FollowedEditorAnchor) => {
		if (flowScriptPanelOpenRef.current) {
			setFlowScriptRevealRequest({ nodeId: anchor.id, token: Date.now() });
			return;
		}
		if (anchor.kind !== "variable") focusNode(anchor.id);
	};
	const [logNodeIdFilter, setLogNodeIdFilter] = useState<string | undefined>();
	const [searchOpen, setSearchOpen] = useState(false);
	const [copilotWorkspaceVisible, setCopilotWorkspaceVisible] = useState(false);
	const [copilotInitialPrompt, setCopilotInitialPrompt] = useState<
		string | undefined
	>();
	// Stable snapshot of the selected flow-node ids handed to FlowPilot. Kept as
	// state (updated only on real selection changes) so the copilot subtree is not
	// re-rendered on every unrelated FlowBoard render.
	const [selectedNodeIds, setSelectedNodeIds] = useState<string[]>([]);
	// Nodes *and* comments — the selection a move to another file would carry.
	const [selectedMovableIds, setSelectedMovableIds] = useState<string[]>([]);

	const handleCopilotClose = useCallback(() => {
		setCopilotInitialPrompt(undefined);
		setCopilotWorkspaceVisible(false);
		surfaceActions.closeSecondary();
	}, [surfaceActions]);
	// Single launcher: hosts with a global assistant (desktop) route to the shared surface store,
	// everything else keeps the embedded FlowCopilot panel.
	const openAssistant = useCallback(
		(prompt?: string) => {
			if (externalAssistant) {
				useAssistantSurface.getState().requestOpenAssistant(prompt);
				return;
			}
			if (prompt) setCopilotInitialPrompt(prompt);
			surfaceActions.openSecondary("flowpilot");
		},
		[externalAssistant, surfaceActions],
	);
	const handleClearRunContext = useCallback(
		() => setCurrentMetadata(undefined),
		[setCurrentMetadata],
	);

	useEffect(() => {
		if (shell.secondary !== "flowpilot") setCopilotWorkspaceVisible(false);
	}, [shell.secondary]);

	const toggleVars = useCallback(
		() => surfaceActions.toggleSidebar("variables"),
		[surfaceActions],
	);
	const toggleLogs = useCallback(
		() => surfaceActions.togglePanel("traces"),
		[surfaceActions],
	);
	const toggleRunHistory = useCallback(
		() => surfaceActions.togglePanel("runs"),
		[surfaceActions],
	);
	const toggleTests = useCallback(
		() => surfaceActions.togglePanel("tests"),
		[surfaceActions],
	);
	const togglePages = useCallback(
		() => surfaceActions.toggleSidebar("explorer"),
		[surfaceActions],
	);

	// A run has traces to show — surface them, but never take over a phone screen.
	useEffect(() => {
		if (!currentMetadata || isMobile) return;
		surfaceActions.openPanel("traces");
	}, [currentMetadata, isMobile, surfaceActions]);

	const toggleFlowScript = useCallback(() => {
		// The rail/palette entry always opens the whole board, never a stale scope.
		setFlowScriptScope(undefined);
		surfaceActions.toggleScript();
	}, [surfaceActions]);

	// "Edit selection as FlowScript": open the panel on a selection-scoped render.
	const openFlowScriptForSelection = useCallback(() => {
		if (selectedNodeIds.length === 0) return;
		setFlowScriptScope([...selectedNodeIds]);
		surfaceActions.openScript();
	}, [selectedNodeIds, surfaceActions]);

	// Join a teammate's shared scoped session (presence bar action): validate
	// their broadcast node ids against the local board — peers are untrusted and
	// nodes may be gone — then open the panel on an independent COPY of the
	// surviving scope. The peer exiting their session never affects this one.
	const joinFlowScriptScope = useCallback(
		(nodeIds: string[]) => {
			const known = resolveJoinableScopeNodeIds(nodeIds, (nodeId) =>
				Boolean(boardRef.current?.nodes?.[nodeId]),
			);
			if (known.length === 0) {
				toastError(
					t("flowscriptScopeGone", "That selection no longer exists"),
					<XIcon />,
				);
				return;
			}
			setFlowScriptScope(known);
			surfaceActions.openScript();
		},
		[surfaceActions, t],
	);

	// Canvas "Go to code" (anchored comment toolbar): open/reveal the FlowScript
	// panel at the comment's anchor. The reveal request retries until the
	// freshly opened panel has rendered the anchor.
	const openFlowScriptAtNode = useCallback(
		(nodeId: string) => {
			surfaceActions.openScript();
			setFlowScriptRevealRequest({ nodeId, token: Date.now() });
		},
		[surfaceActions],
	);
	const openFlowScriptAtNodeRef = useRef(openFlowScriptAtNode);
	openFlowScriptAtNodeRef.current = openFlowScriptAtNode;

	// Transient FlowScript-cursor highlight: a DOM class toggle on the rendered
	// node, so it never touches selection, focus or the viewport. Nodes on other
	// layers have no DOM element and simply stay unhighlighted; the explicit
	// "Reveal on board" action goes through focusNode instead.
	const flowScriptHighlightRef = useRef<string | undefined>(undefined);
	const highlightNodeOnCanvas = useCallback((nodeId?: string) => {
		const previous = flowScriptHighlightRef.current;
		if (previous === nodeId) return;
		if (previous) {
			document
				.querySelector(`.react-flow__node[data-id="${previous}"]`)
				?.classList.remove("flowscript-nav-highlight");
		}
		flowScriptHighlightRef.current = nodeId;
		if (nodeId) {
			document
				.querySelector(`.react-flow__node[data-id="${nodeId}"]`)
				?.classList.add("flowscript-nav-highlight");
		}
	}, []);

	// Sections a full FlowScript render consists of: event entry nodes + function layers.
	const totalFlowScriptSections = useMemo(() => {
		const data = board.data;
		if (!data) return undefined;
		const eventSections = Object.values(data.nodes).filter(
			(node) => node.start,
		).length;
		const functionSections = Object.values(data.layers ?? {}).filter(
			(layer) => layer.type === ILayerType.Function,
		).length;
		return eventSections + functionSections;
	}, [board.data]);

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

	const buildRuntimeVariablesMap = useCallback(
		(
			storedValues: Map<string, RuntimeVariableValue>,
			isRemote: boolean,
		): Record<string, IVariable> | undefined => {
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
			return Object.keys(runtimeVariables).length > 0
				? runtimeVariables
				: undefined;
		},
		[runtimeConfiguredVars],
	);

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
				return {
					intercepted: false,
					runtimeVariables: buildRuntimeVariablesMap(
						storedValues,
						isRemote ?? false,
					),
				};
			}

			// Need to prompt for configuration
			const existingValues = await runtimeVarsContext.getValues(appId);
			setExistingRuntimeVars(existingValues);
			setPendingExecution((previous) => {
				previous?.cancel?.();
				return { node, payload, isRemote: isRemote ?? false };
			});
			setRuntimeVarsPromptOpen(true);
			return { intercepted: true }; // Intercepted
		},
		[
			appId,
			runtimeConfiguredVars,
			runtimeVarsContext,
			buildRuntimeVariablesMap,
		],
	);

	// Cancel runtime vars prompt
	const handleRuntimeVarsCancel = useCallback(() => {
		setRuntimeVarsPromptOpen(false);
		pendingExecution?.cancel?.();
		setPendingExecution(null);
	}, [pendingExecution]);

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
					errorMessage || t("failedToExecuteBoard", "Failed to execute board"),
					<PlayCircleIcon className="w-4 h-4" />,
				);
				return;
			}
			removeRun(runId);
			if (!meta && !runId) {
				toastError(
					t("failedToExecuteBoard", "Failed to execute board"),
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
					t("remoteExecutionNotAvailable", "Remote execution not available"),
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
					errorMessage ||
						t(
							"failedToExecuteBoardOnServer",
							"Failed to execute board on server",
						),
					<PlayCircleIcon className="w-4 h-4" />,
				);
				return;
			}
			removeRun(runId);
			if (!meta && !runId) {
				toastError(
					t(
						"failedToExecuteBoardOnServer",
						"Failed to execute board on server",
					),
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
			const { node, payload, isRemote, resume } = pendingExecution;
			setPendingExecution(null);

			const varsMap =
				Object.keys(runtimeVariables).length > 0 ? runtimeVariables : undefined;
			if (resume) {
				resume(varsMap);
			} else if (isRemote) {
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

	// One pre-flight for a batch of test runs: WASM consent plus runtime
	// variables, prompting through the existing dialog when values are missing.
	const prepareTestRun = useCallback(
		async (
			representative: INode,
		): Promise<{
			ok: boolean;
			runtimeVariables?: Record<string, IVariable>;
		}> => {
			const wasmOk = await checkWasmConsent();
			if (!wasmOk) return { ok: false };
			if (runtimeConfiguredVars.length === 0 || !runtimeVarsContext) {
				return { ok: true };
			}
			const hasAll = await runtimeVarsContext.hasAllValues(
				appId,
				runtimeConfiguredVars.map((v) => v.id),
			);
			if (!hasAll) {
				const existingValues = await runtimeVarsContext.getValues(appId);
				setExistingRuntimeVars(existingValues);
				return new Promise((resolve) => {
					setPendingExecution((previous) => {
						previous?.cancel?.();
						return {
							node: representative,
							isRemote: false,
							resume: (runtimeVariables) =>
								resolve({ ok: true, runtimeVariables }),
							cancel: () => resolve({ ok: false }),
						};
					});
					setRuntimeVarsPromptOpen(true);
				});
			}
			const storedValues = await runtimeVarsContext.getValues(appId);
			return {
				ok: true,
				runtimeVariables: buildRuntimeVariablesMap(storedValues, false),
			};
		},
		[
			appId,
			checkWasmConsent,
			runtimeConfiguredVars,
			runtimeVarsContext,
			buildRuntimeVariablesMap,
		],
	);

	// Raw run for the Tests panel: no toasts — verdicts surface in the panel.
	const executeTestNode = useCallback(
		async (
			node: INode,
			runtimeVariables?: Record<string, IVariable>,
		): Promise<ILogMetadata | undefined> => {
			const startedAtMicros = Date.now() * 1000;
			let added = false;
			let runId = "";
			let meta: ILogMetadata | undefined;
			try {
				meta = await backend.boardState.executeBoard(
					appId,
					boardId,
					{
						id: node.id,
						payload: {},
						runtime_variables: runtimeVariables,
					},
					true,
					(id: string) => {
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
							added = true;
							addRun(firstItem.runId, boardId, [node.id]);
						}
						pushUpdate(firstItem.runId, runUpdates);
					},
				);
			} catch (error) {
				// Same recovery events the Run button dispatches, so consent
				// dialogs still open; the test itself reports the error.
				const oauthError = error as Error & {
					isOAuthError?: boolean;
					missingProviders?: unknown[];
				};
				if (oauthError.isOAuthError && oauthError.missingProviders) {
					window.dispatchEvent(
						new CustomEvent("flow:oauth-required", {
							detail: {
								missingProviders: oauthError.missingProviders,
								appId,
								boardId,
								nodeId: node.id,
								payload: {},
							},
						}),
					);
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
								payload: {},
								permissions: rpaPermissionError.permissions,
							},
						}),
					);
				}
				throw error;
			} finally {
				if (runId) removeRun(runId);
			}
			if (meta || !runId) return meta;
			// Remote backends resolve without metadata — recover it by run id so
			// the run can still be graded.
			const runs = await backend.boardState.listRuns(
				appId,
				boardId,
				undefined,
				startedAtMicros - 60_000_000,
				undefined,
				undefined,
				undefined,
				0,
				100,
			);
			return runs.find((run) => run.run_id === runId);
		},
		[appId, boardId, backend, addRun, pushUpdate, removeRun],
	);

	const openTestRunLogs = useCallback(
		(meta: ILogMetadata) => {
			setCurrentMetadata(meta);
			surfaceActions.openPanel("traces");
		},
		[setCurrentMetadata, surfaceActions],
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
				toastError(
					t("cannotChangeOldVersion", "Cannot change old version"),
					<XIcon />,
				);
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
			const prompt = t(
				"explainWhatTheseCountSelectedNodesDoAndHowTheyWorkTogetherInThisFlow",
				{
					defaultValue_one:
						"Explain what this node does and how it works in the context of this flow.",
					defaultValue_other:
						"Explain what these {{count}} selected nodes do and how they work together in this flow.",
					count: nodeCount,
				},
			);

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

	// Advisory collision toast (FlowScript collab rule 3): an undo/redo batch
	// that touches statements a peer is editing in the code view still applies
	// (last-writer-wins) but names the collision. Claims are read straight from
	// awareness — one cheap sanitized pass, only when history actually fires.
	const warnOnHistoryClaimCollision = useCallback(
		(commands: IGenericCommand[]) => {
			if (!awareness) return;
			const entityIds = collectCommandEntityIds(commands);
			if (entityIds.size === 0) return;
			const hit = findClaimCollision(
				readPeerFlowScriptClaims(awareness, sub),
				entityIds,
			);
			if (!hit) return;
			const name =
				(hit.sub ? peerUsers?.get(hit.sub)?.truncatedName : undefined) ??
				t("common:user", "User");
			toastWarning(
				t("flowscriptEditCollision", {
					defaultValue: "This change touches statements {{name}} is editing",
					name,
				}),
				<PencilLineIcon className="w-4 h-4" />,
			);
		},
		[awareness, sub, peerUsers, t],
	);

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
		onHistoryBatch: warnOnHistoryClaimCollision,
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

	const handleFilterLogs = useCallback(
		(nodeId: string) => {
			setLogNodeIdFilter(nodeId);
			surfaceActions.openPanel("traces");
		},
		[surfaceActions],
	);
	const handleFilterLogsRef = useRef(handleFilterLogs);
	handleFilterLogsRef.current = handleFilterLogs;

	// Extract stable primitives from complex objects to avoid re-parsing on unrelated changes
	const connectionMode =
		currentProfile.data?.settings?.connection_mode ?? "default";
	const isOffline = app.data?.visibility === IAppVisibility.Offline;
	const hasRemoteExecution = !!backend.boardState.executeBoardRemote;

	// What each event entry node may do, for the FlowScript run lenses. Same
	// derivation as the canvas play button (deriveRunCapabilities), so the two
	// surfaces can never disagree.
	const runnableEventNodes = useMemo(() => {
		if (!board.data) return undefined;
		const map = new Map<string, FlowScriptRunCapability>();
		for (const node of Object.values(board.data.nodes)) {
			if (!node.start) continue;
			const capabilities = deriveRunCapabilities({
				executionMode: board.data.execution_mode,
				isOffline,
				hasRemoteExecute: hasRemoteExecution,
				onlyOffline: node.only_offline,
			});
			map.set(node.id, {
				local: capabilities.canLocalExecute,
				remote: capabilities.canRemoteExecute,
			});
		}
		return map;
	}, [board.data, isOffline, hasRemoteExecution]);

	// FlowScript "▶ Run" lens controller. Payload-less events run through the
	// exact gates the canvas play button uses (executeBoard/executeBoardRemote:
	// WASM consent → runtime vars → internal execute); events with output pins
	// open the same EventPayloadForm, hosted in a board-level dialog.
	const [runDialogNodeId, setRunDialogNodeId] = useState<string | undefined>(
		undefined,
	);
	const runDialogBusyRef = useRef(false);
	const onRunEventNode = useCallback(
		async (nodeId: string, mode: FlowScriptRunMode) => {
			const node = boardRef.current?.nodes[nodeId];
			if (!node?.start) return;
			const capability = runnableEventNodes?.get(nodeId);
			if (mode === "local" && capability?.local !== true) return;
			if (mode === "remote" && capability?.remote !== true) return;
			if (Object.keys(node.pins).length <= 1) {
				if (mode === "remote") await executeBoardRemote(node);
				else await executeBoard(node);
				return;
			}
			setRunDialogNodeId(nodeId);
		},
		[runnableEventNodes, executeBoard, executeBoardRemote],
	);
	const runDialogNode = runDialogNodeId
		? board.data?.nodes[runDialogNodeId]
		: undefined;
	const runDialogCapability = runDialogNodeId
		? runnableEventNodes?.get(runDialogNodeId)
		: undefined;
	const closeRunDialog = useCallback(() => setRunDialogNodeId(undefined), []);
	const runDialogLocalExecute = useCallback(
		async (payload?: object) => {
			const node = runDialogNodeId
				? boardRef.current?.nodes[runDialogNodeId]
				: undefined;
			if (!node || runDialogBusyRef.current) return;
			runDialogBusyRef.current = true;
			try {
				await executeBoard(node, payload);
			} finally {
				runDialogBusyRef.current = false;
			}
		},
		[runDialogNodeId, executeBoard],
	);
	const runDialogRemoteExecute = useCallback(
		async (payload?: object) => {
			const node = runDialogNodeId
				? boardRef.current?.nodes[runDialogNodeId]
				: undefined;
			if (!node || runDialogBusyRef.current) return;
			runDialogBusyRef.current = true;
			try {
				await executeBoardRemote(node, payload);
			} finally {
				runDialogBusyRef.current = false;
			}
		},
		[runDialogNodeId, executeBoardRemote],
	);

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
			(comment: IComment) => {
				if (comment.node_id) openFlowScriptAtNodeRef.current(comment.node_id);
			},
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
		// The minimap SVG repaints on every pan/zoom frame; WebKit rasterizes
		// oklch color-mix fills slowly enough that a large board stutters, so it
		// gets flat token colors while Chromium keeps the translucent tint.
		const tint = (token: string, percent: number) =>
			isWebkitLite()
				? `var(${token})`
				: `color-mix(in oklch, var(${token}) ${percent}%, transparent)`;

		if (node.type === "layerNode") return tint("--foreground", 50);

		if (node.type === "node") {
			const nodeData: INode = node.data.node as INode;
			if (nodeData.event_callback || nodeData.start)
				return tint("--primary", 80);
			if (
				!Object.values(nodeData.pins).find(
					(pin) => pin.data_type === IVariableType.Execution,
				)
			) {
				return tint("--tertiary", 80);
			}
			return tint("--muted", 80);
		}
		if (node.type === "commentNode") {
			const commentData: IComment = node.data.comment as IComment;
			let color = commentData.color ?? tint("--muted", 80);

			if (color.startsWith("#")) {
				color = hexToRgba(color, 0.3);
			}
			return color;
		}
		return tint("--primary", 60);
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
			const movableIds = selectedNodes
				.filter((selectedNode) =>
					MOVABLE_SELECTION_TYPES.has(selectedNode.type ?? ""),
				)
				.map((selectedNode) => selectedNode.id);
			setSelectedNodeIds((prev) => (sameIds(prev, nodeIds) ? prev : nodeIds));
			setSelectedMovableIds((prev) =>
				sameIds(prev, movableIds) ? prev : movableIds,
			);
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

	/**
	 * Re-files the selection into another module — `null` is `main.flow`, the board root.
	 * Which file an event belongs to follows its ENTRY node, so moving part of a chain
	 * changes where those nodes are drawn, not the event's file assignment.
	 */
	const moveSelectionToModule = useCallback(
		async (target: string | null) => {
			if (selectedMovableIds.length === 0) return;
			await executeCommand(
				moveToLayerCommand({ ids: selectedMovableIds, target }),
				false,
			);
		},
		[executeCommand, selectedMovableIds],
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
								? t("functionReferenceIn", "Function Reference In")
								: t("functionReferenceOut", "Function Reference Out"),
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
			toastError(t("noNodesToCapture", "No nodes to capture"), <XIcon />);
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
				toastSuccess(
					t("screenshotCopiedToClipboard", "Screenshot copied to clipboard"),
					<CheckIcon />,
				);
			} catch {
				const link = document.createElement("a");
				link.download = "flow-screenshot.png";
				link.href = dataUrl;
				link.click();
				toastSuccess("Screenshot downloaded", <CheckIcon />);
			}
		} catch (error) {
			console.error("Screenshot failed:", error);
			toastError(
				t("failedToCaptureScreenshot", "Failed to capture screenshot"),
				<XIcon />,
			);
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

	// FlowScript comment bridge: the editor mutates board comments through the
	// same command funnel as the canvas (undo-able, sync-propagated). A comment
	// carries its target layer; the upsert routes it via current_layer exactly
	// like the canvas path does for the layer it renders in.
	const onUpsertComment = useCallback(
		async (comment: IComment) => {
			await executeCommand(
				upsertCommentCommand({ comment, current_layer: comment.layer ?? null }),
			);
		},
		[executeCommand],
	);
	const onRemoveComment = useCallback(
		async (comment: IComment) => {
			await executeCommand(removeCommentCommand({ comment }));
		},
		[executeCommand],
	);
	// Position/layer of an anchored node so an editor-created comment lands
	// next to it on the canvas.
	const getNodeSpatial = useCallback((nodeId: string) => {
		const node = boardRef.current?.nodes[nodeId];
		if (!node) return undefined;
		return {
			coordinates: node.coordinates ?? undefined,
			layer: node.layer ?? undefined,
		};
	}, []);

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
				toastError(
					t("nodeTypeNode_typeNotFound", "Node type {{node_type}} not found", {
						node_type: suggestion.node_type,
					}),
					<XIcon />,
				);
			}
		},
		[catalog.data, placeNode],
	);

	const [autoLayoutDialogOpen, setAutoLayoutDialogOpen] = useState(false);

	const autoLayout = useCallback(
		async (style: LayoutStyle = "compact") => {
			if (typeof version !== "undefined") {
				toastError(
					t("cannotModifyOldVersion", "Cannot modify old version"),
					<XIcon />,
				);
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
					// Modules are virtual files, never chips on the canvas — laying one out
					// would reserve space for a box that is not drawn.
					if (layer.type === ILayerType.Module) continue;
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

			// Inside a layer, its own boundary nodes are real, wired, movable nodes
			// (flow-board-utils renders them with inverted pins). Leaving them out
			// reflows the whole body around two anchors the layout never saw.
			const openLayer = currentLayer
				? boardData.layers?.[currentLayer]
				: undefined;
			if (openLayer) {
				const inputPins: Record<string, IPin> = {};
				const returnPins: Record<string, IPin> = {};
				for (const pin of Object.values(openLayer.pins)) {
					const inverted: IPin = {
						...pin,
						pin_type:
							pin.pin_type === IPinType.Input
								? IPinType.Output
								: IPinType.Input,
					};
					if (inverted.pin_type === IPinType.Output)
						inputPins[inverted.id] = inverted;
					else returnPins[inverted.id] = inverted;
				}

				const boundary = (
					suffix: "-input" | "-return",
					pins: Record<string, IPin>,
					coordinates: number[] | null | undefined,
					isStart: boolean,
				): INode =>
					({
						id: openLayer.id + suffix,
						category: "",
						coordinates: [coordinates?.[0] ?? 0, coordinates?.[1] ?? 0, 0],
						description: "",
						event_callback: false,
						friendly_name: openLayer.name,
						fn_refs: null,
						name: openLayer.id + suffix,
						pins,
						start: isStart,
					}) as unknown as INode;

				// A pinless boundary node has no edges, so it would be packed as a
				// stray island — and, being a start node, would then anchor the whole
				// layer to that stray position. Only include the ones that are wired.
				if (Object.keys(inputPins).length > 0) {
					layerNodes.push(
						boundary("-input", inputPins, openLayer.in_coordinates, true),
					);
				}
				if (Object.keys(returnPins).length > 0) {
					layerNodes.push(
						boundary("-return", returnPins, openLayer.out_coordinates, false),
					);
				}
			}

			if (layerNodes.length === 0 && layerEntities.length === 0) return;

			// Real rendered sizes beat any formula: columns are spaced by the
			// widest node they contain and rows by real heights, so a mis-measured
			// node is the difference between a clean board and overlapping nodes.
			const nodeSizes = new Map<string, [number, number]>();
			for (const rendered of getNodes()) {
				const width = rendered.measured?.width ?? rendered.width;
				const height = rendered.measured?.height ?? rendered.height;
				if (
					typeof width === "number" &&
					typeof height === "number" &&
					width >= 8 &&
					height >= 8
				) {
					nodeSizes.set(rendered.id, [width, height]);
				}
			}

			// Comments are annotations over a region of the graph. They move with
			// the nodes they cover, otherwise every layout strands them.
			const comments: LayoutComment[] = [];
			for (const comment of Object.values(boardData.comments ?? {})) {
				const commentLayer =
					(comment.layer ?? "") === "" ? undefined : comment.layer;
				if (commentLayer !== currentLayer) continue;
				comments.push({
					id: comment.id,
					x: comment.coordinates[0] ?? 0,
					y: comment.coordinates[1] ?? 0,
					width: comment.width ?? 200,
					height: comment.height ?? 200,
					isLocked: comment.is_locked === true,
				});
			}

			// Laying out just the selection keeps the rest of the board where the
			// user left it, which is what makes this safe to press.
			const scoped = selectedNodeIds.length > 1;
			const only = scoped ? new Set(selectedNodeIds) : undefined;

			// A scoped layout only reasons about the selection, so hand it the
			// boxes it must stay off.
			const obstacles: LayoutBox[] = [];
			if (only) {
				for (const rendered of getNodes()) {
					if (only.has(rendered.id)) continue;
					const size = nodeSizes.get(rendered.id);
					if (!size) continue;
					obstacles.push({
						x: rendered.position.x,
						y: rendered.position.y,
						width: size[0],
						height: size[1],
					});
				}
			}

			const { positions, commentPositions } = computeFlowLayoutDetailed(
				{
					layerNodes,
					layerEntities,
					boardLayers: boardData.layers,
					currentLayer,
					nodeSizes,
					comments: scoped ? [] : comments,
					only,
					obstacles,
				},
				style,
			);

			const commands: IGenericCommand[] = [];
			for (const node of layerNodes) {
				const pos = positions.get(node.id);
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
				const pos = positions.get(entity.id);
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
			for (const comment of comments) {
				const pos = commentPositions.get(comment.id);
				if (!pos) continue;
				if (pos[0] === comment.x && pos[1] === comment.y) continue;
				commands.push(
					moveNodeCommand({
						node_id: comment.id,
						from_coordinates: [comment.x, comment.y, 0],
						to_coordinates: [pos[0], pos[1], 0],
						current_layer: currentLayer,
					}),
				);
			}

			if (commands.length === 0) return;
			await executeCommands(commands);

			setTimeout(
				() =>
					fitView({
						padding: 0.2,
						duration: 300,
						nodes: scoped ? selectedNodeIds.map((id) => ({ id })) : undefined,
					}),
				100,
			);
		},
		[
			board.data,
			currentLayer,
			executeCommands,
			fitView,
			getNodes,
			selectedNodeIds,
			version,
		],
	);

	// Use the copilot commands hook for executing AI-generated commands
	const { handleExecuteCommands } = useCopilotCommands({
		board,
		catalog,
		executeCommands,
		currentLayer,
	});
	// A per-file editor names the file it applies; that name IS the module identity and the layer
	// the apply runs in (main has neither). Without a file the apply keeps using the open layer.
	const handleApplyFlowScript = useCallback(
		(flowscript: string, options?: FlowScriptApplyOptions) => {
			const moduleId = fileModuleId(options?.file);
			return applyFlowScript(
				flowscript,
				options?.file ? moduleId : currentLayer,
				catalog.data,
				{ ...options, module: moduleId },
			);
		},
		[applyFlowScript, currentLayer, catalog.data],
	);
	const handleApplyFlowIrCommit = useCallback(
		(
			token: Parameters<typeof applyFlowIrCommit>[0],
			deliveryId?: string,
			historyMode?: Parameters<typeof applyFlowIrCommit>[2],
		) => applyFlowIrCommit(token, deliveryId, historyMode),
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

	// One registry behind the rail, the chords and the Spotlight palette, so a
	// board action cannot exist in one of the three and be missing from the others.
	const boardCommands = useMemo<IBoardCommand[]>(
		() => [
			{
				// Survives layers and module tabs, and falls back to the app's flow
				// list. Gating it on a registered parent left every entry point that
				// is not the flows overview — Spotlight, deeplinks, FlowPilot — with
				// no exit at all, and the board owns the window there.
				//
				// No chord: ⌘B already places a Branch node, and that handler is on
				// `document`, upstream of this registry's `window` listener, so it
				// stops propagation before the command could ever see the event.
				id: "back",
				surface: "rail",
				title: boardParent ? t("backToApp", "Back to app") : t("home", "Home"),
				icon: boardParent ? ArrowBigLeftDashIcon : HouseIcon,
				when: canNavigateOut,
				run: () => router.push(boardParent ?? appHref),
			},
			{
				id: "explorer",
				surface: "rail",
				title: t("explorer", "Explorer"),
				icon: FilesIcon,
				shortcut: "mod+shift+e",
				run: togglePages,
			},
			{
				id: "search",
				surface: "rail",
				title: t("searchBoard", "Search board"),
				icon: SearchIcon,
				shortcut: "mod+shift+f",
				run: () => surfaceActions.toggleSidebar("search"),
			},
			{
				id: "search-dialog",
				surface: "palette",
				title: t("findOnBoard", "Find on board"),
				icon: SearchIcon,
				shortcut: "mod+f",
				run: () => setSearchOpen(true),
			},
			{
				id: "variables",
				surface: "rail",
				title: t("variablesFunctions", "Variables & Functions"),
				icon: VariableIcon,
				shortcut: "mod+shift+v",
				run: toggleVars,
			},
			{
				id: "events",
				surface: "rail",
				title: t("entryPoints", "Entry points"),
				icon: ZapIcon,
				run: () => surfaceActions.toggleSidebar("events"),
			},
			{
				id: "comments",
				surface: "rail",
				title: t("comments", "Comments"),
				icon: MessageSquareIcon,
				run: () => surfaceActions.toggleSidebar("comments"),
			},
			{
				id: "flowscript",
				surface: "editor",
				title: t("flowscript", "FlowScript"),
				icon: FileCode2Icon,
				shortcut: "mod+\\",
				run: toggleFlowScript,
			},
			{
				id: "runs",
				surface: "rail",
				title: t("runHistory", "Run History"),
				icon: HistoryIcon,
				shortcut: "mod+shift+r",
				run: toggleRunHistory,
			},
			{
				id: "traces",
				surface: "rail",
				title: t("logs", "Logs"),
				icon: ScrollIcon,
				shortcut: "mod+j",
				run: toggleLogs,
			},
			{
				id: "tests",
				surface: "rail",
				title: t("tests", "Tests"),
				icon: FlaskConicalIcon,
				run: toggleTests,
			},
			{
				id: "inspector",
				surface: "rail-bottom",
				title: t("nodeInfo", "Node Info"),
				icon: SlidersHorizontalIcon,
				shortcut: "mod+alt+i",
				run: () => surfaceActions.toggleSecondary("inspector"),
			},
			{
				id: "templates",
				surface: "editor",
				title: t("templates", "Templates"),
				icon: LayoutTemplateIcon,
				run: () => setTemplateSelectorOpen(true),
			},
			{
				id: "auto-layout",
				surface: "editor",
				title: t("autoLayout", "Auto Layout"),
				icon: WaypointsIcon,
				run: () => setAutoLayoutDialogOpen(true),
			},
			{
				id: "layer-up",
				surface: "rail-bottom",
				title: t("layerUp", "Layer Up"),
				icon: SquareChevronUpIcon,
				when: Boolean(currentLayer) && !insideModule,
				run: () => popLayer(),
			},
			{
				id: "flowpilot",
				// Bubble hosts already have an entry point, so the rail shows nothing —
				// but the chord and the palette still reach the assistant there.
				surface: externalAssistant ? "palette" : "rail-bottom",
				title: t("flowpilot", "FlowPilot"),
				icon: SparklesIcon,
				shortcut: "mod+alt+b",
				run: () => openAssistant(),
			},
		],
		[
			t,
			router,
			boardParent,
			appHref,
			boardId,
			togglePages,
			toggleVars,
			toggleFlowScript,
			toggleRunHistory,
			toggleLogs,
			surfaceActions,
			currentLayer,
			insideModule,
			popLayer,
			openAssistant,
			externalAssistant,
		],
	);
	useBoardCommands(boardCommands);

	// Which surface a command shows on is declared on the command itself, so the
	// rail and the editor strip are derived rather than hand-listed — adding a
	// command cannot leave it reachable only by search.
	const isCommandActive = useCallback(
		(id: string) => {
			switch (id) {
				case "flowscript":
					return shell.script;
				case "runs":
				case "traces":
				case "tests":
				case "problems":
					return shell.panel === id;
				case "inspector":
				case "flowpilot":
					return shell.secondary === id;
				default:
					return shell.sidebar === id;
			}
		},
		[shell],
	);

	const railItems = useMemo<IBoardRailItem[]>(
		() =>
			commandsFor(boardCommands, "rail").map((command) => {
				const Icon = command.icon;
				return {
					id: command.id,
					title: command.title,
					icon: Icon ? <Icon /> : null,
					shortcut: command.shortcut
						? formatShortcut(command.shortcut)
						: undefined,
					active: isCommandActive(command.id),
					badge:
						command.id === "comments"
							? Object.keys(board.data?.comments ?? {}).length
							: undefined,
					onSelect: command.run,
				};
			}),
		[boardCommands, isCommandActive, board.data?.comments],
	);

	// Actions on the open document, beside the file tabs.
	const editorActions = useMemo<IBoardEditorAction[]>(
		() =>
			commandsFor(boardCommands, "editor").map((command) => {
				const Icon = command.icon;
				const isScript = command.id === "flowscript";
				return {
					id: command.id,
					title: command.title,
					label: command.title,
					icon: isScript ? <Columns2Icon /> : Icon ? <Icon /> : null,
					shortcut: command.shortcut
						? formatShortcut(command.shortcut)
						: undefined,
					active: isCommandActive(command.id),
					// A published version is read-only, so document mutations are off;
					// opening the script beside it still is not a mutation.
					disabled: !isScript && typeof version !== "undefined",
					onSelect: command.run,
				};
			}),
		[boardCommands, isCommandActive, version],
	);

	const railBottomItems = useMemo<IBoardRailItem[]>(() => {
		const items: IBoardRailItem[] = commandsFor(
			boardCommands,
			"rail-bottom",
		).map((command) => {
			const Icon = command.icon;
			return {
				id: command.id,
				title: command.title,
				icon: Icon ? <Icon /> : null,
				shortcut: command.shortcut
					? formatShortcut(command.shortcut)
					: undefined,
				active: isCommandActive(command.id),
				onSelect: command.run,
			};
		});
		// Host-provided entries (the desktop RPA recorder) keep their place now
		// that the dock they used to live in is gone. They are not board commands,
		// so they are appended rather than registered.
		for (const [index, item] of (extraDockItems ?? []).entries()) {
			items.push({
				id: `host-${index}`,
				title: item.title,
				icon: item.icon,
				active: item.highlight,
				onSelect: () => void item.onClick(),
			});
		}
		return items;
	}, [boardCommands, isCommandActive, extraDockItems]);

	const problemNodes = useMemo(
		() =>
			Object.values(board.data?.nodes ?? {}).filter((node) =>
				Boolean(node.error),
			),
		[board.data?.nodes],
	);
	const eventNodes = useMemo(
		() => Object.values(board.data?.nodes ?? {}).filter((node) => node.start),
		[board.data?.nodes],
	);
	const boardComments = useMemo(
		() => Object.values(board.data?.comments ?? {}),
		[board.data?.comments],
	);

	const openPageInBuilder = useCallback(
		(pageId: string, bId: string) => {
			router.push(`/page-builder?id=${pageId}&app=${appId}&board=${bId}`);
		},
		[router, appId],
	);

	const sidebarBody =
		shell.sidebar === "explorer" ? (
			<BoardExplorer
				appId={appId}
				boardId={boardId}
				board={board.data}
				currentFileId={currentFileId}
				onSelectFile={handleSelectModule}
				onOpenPage={openPageInBuilder}
				executeCommand={executeCommand}
				readOnly={typeof version !== "undefined"}
				reservedRoots={moduleReservedRoots}
			/>
		) : shell.sidebar === "search" ? (
			<FlowSearch
				board={board.data}
				open
				onOpenChange={(open) => {
					if (!open) surfaceActions.closeSidebar();
				}}
				onNavigate={focusNode}
				mode="sidebar"
			/>
		) : shell.sidebar === "variables" ? (
			board.data && (
				<VariablesMenu
					board={board.data}
					executeCommand={executeCommand}
					currentLayerId={currentLayer}
					pushLayer={pushLayer}
					boardRef={boardRef}
				/>
			)
		) : shell.sidebar === "events" ? (
			<ul className="flex flex-col p-1">
				{eventNodes.length === 0 && (
					<li className="px-2 py-1 text-xs text-muted-foreground">
						{t("noEntryPointsYet", "No entry points yet")}
					</li>
				)}
				{eventNodes.map((node) => (
					<li key={node.id}>
						<button
							type="button"
							onClick={() => focusNode(node.id)}
							className="flex w-full items-center gap-2 rounded-sm px-2 py-1 text-left text-xs hover:bg-accent"
						>
							<ZapIcon className="size-3 shrink-0 text-emerald-500" />
							<span className="truncate">{node.friendly_name}</span>
						</button>
					</li>
				))}
			</ul>
		) : shell.sidebar === "comments" ? (
			<ul className="flex flex-col p-1">
				{boardComments.length === 0 && (
					<li className="px-2 py-1 text-xs text-muted-foreground">
						{t("noCommentsYet", "No comments yet")}
					</li>
				)}
				{boardComments.map((comment) => (
					<li key={comment.id}>
						<button
							type="button"
							onClick={() =>
								comment.node_id
									? openFlowScriptAtNode(comment.node_id)
									: focusNode(comment.id)
							}
							className="flex w-full flex-col gap-0.5 rounded-sm px-2 py-1 text-left hover:bg-accent"
						>
							<span className="line-clamp-3 whitespace-pre-line text-xs">
								{plainTextFromRichContent(comment.content) ||
									t("emptyComment", "Empty comment")}
							</span>
							<span className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
								{comment.author && <span>{comment.author}</span>}
								{comment.node_id && (
									<span className="text-primary">
										{board.data?.nodes?.[comment.node_id]?.friendly_name ??
											t("node", "Node:")}
									</span>
								)}
							</span>
						</button>
					</li>
				))}
			</ul>
		) : null;

	const SIDEBAR_TITLES: Record<string, string> = {
		explorer: t("explorer", "Explorer"),
		search: t("search", "Search"),
		variables: t("variablesFunctions", "Variables & Functions"),
		events: t("entryPoints", "Entry points"),
		comments: t("comments", "Comments"),
	};

	const MOBILE_TITLES: Record<string, string> = {
		...SIDEBAR_TITLES,
		problems: t("problems", "Problems"),
		runs: t("runs", "Runs"),
		traces: t("logs", "Logs"),
		tests: t("tests", "Tests"),
		script: t("flowscript", "FlowScript"),
		inspector: t("nodeInfo", "Node Info"),
		flowpilot: t("flowpilot", "FlowPilot"),
	};

	const panelBody =
		shell.panel === "runs" ? (
			board.data && (
				<FlowRuns
					executeBoard={executeBoard}
					nodes={board.data.nodes}
					appId={appId}
					boardId={boardId}
					version={board.data.version as [number, number, number]}
					onVersionChange={setVersion}
					onFocusNode={focusNode}
					variant="panel"
				/>
			)
		) : shell.panel === "traces" ? (
			board.data && currentMetadata ? (
				<Traces
					appId={appId}
					boardId={boardId}
					board={boardRef}
					onFocusNode={focusNode}
					nodeIdFilter={logNodeIdFilter}
					onClearNodeIdFilter={() => setLogNodeIdFilter(undefined)}
					variant="panel"
				/>
			) : (
				<p className="p-3 text-xs text-muted-foreground">
					{t("noLogs", "No Logs")}
				</p>
			)
		) : shell.panel === "tests" ? (
			board.data && (
				<FlowTests
					appId={appId}
					boardId={boardId}
					nodes={board.data.nodes}
					onFocusNode={focusNode}
					onOpenRunLogs={openTestRunLogs}
					prepareRun={prepareTestRun}
					executeTest={executeTestNode}
					variant="panel"
				/>
			)
		) : (
			<ul className="flex h-full flex-col overflow-auto p-1">
				{problemNodes.length === 0 && (
					<li className="flex h-full flex-col items-center justify-center gap-1 text-center">
						<CheckIcon className="size-5 text-muted-foreground/60" />
						<p className="text-sm font-medium">
							{t("noProblems", "No problems")}
						</p>
						<p className="text-xs text-muted-foreground">
							{t(
								"nothingOnThisBoardReportsAnError",
								"Nothing on this board reports an error.",
							)}
						</p>
					</li>
				)}
				{problemNodes.map((node) => (
					<li key={node.id}>
						<button
							type="button"
							onClick={() => focusNode(node.id)}
							className="flex w-full items-center gap-2 rounded-sm px-2 py-1 text-left text-xs hover:bg-accent"
						>
							<TriangleAlertIcon className="size-3 shrink-0 text-destructive" />
							<span className="shrink-0 font-medium">{node.friendly_name}</span>
							<span className="truncate text-muted-foreground">
								{node.error}
							</span>
						</button>
					</li>
				))}
			</ul>
		);

	const scriptPane =
		board.data && shell.script ? (
			<FlowScriptPanel
				appId={appId}
				boardId={boardId}
				version={version}
				boardUpdatedAt={board.dataUpdatedAt}
				catalogNodes={catalog.data}
				selectedNodeIds={selectedNodeIds}
				onHighlightNode={highlightNodeOnCanvas}
				onRevealNode={focusNode}
				scopeNodeIds={flowScriptScope}
				onExitScope={() => setFlowScriptScope(undefined)}
				modules={modules}
				currentFile={currentFileId}
				onSelectFile={handleSelectModule}
				files={flowScriptFiles}
				boardScope={flowScriptBoardScope}
				totalSections={totalFlowScriptSections}
				onApplyFlowScript={handleApplyFlowScript}
				onClose={surfaceActions.closeScript}
				awareness={awareness}
				sub={sub}
				peerUsers={peerUsers}
				revealRequest={flowScriptRevealRequest}
				onRunEventNode={onRunEventNode}
				runnableEventNodes={runnableEventNodes}
				remoteExecutions={remoteExecutions}
				comments={board.data.comments}
				onUpsertComment={onUpsertComment}
				onRemoveComment={onRemoveComment}
				getNodeSpatial={getNodeSpatial}
			/>
		) : undefined;

	const secondaryPane =
		shell.secondary === "inspector" ? (
			<BoardPane
				title={t("nodeInfo", "Node Info")}
				onClose={surfaceActions.closeSecondary}
			>
				<BoardInspector
					board={board.data}
					selectedNodeIds={selectedNodeIds}
					onRevealNode={focusNode}
				/>
			</BoardPane>
		) : shell.secondary === "flowpilot" && !externalAssistant ? (
			<BoardPane
				title={t("flowpilot", "FlowPilot")}
				onClose={() => {
					surfaceActions.closeSecondary();
					handleCopilotClose();
				}}
				bodyClassName="overflow-hidden"
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
					onClose={() => {
						surfaceActions.closeSecondary();
						handleCopilotClose();
					}}
					onWorkspaceVisibleChange={setCopilotWorkspaceVisible}
					mode="panel"
					initialPrompt={copilotInitialPrompt}
				/>
			</BoardPane>
		) : undefined;

	const layerBreadcrumb = currentLayer
		? insideModule
			? modulePathLabel(board.data?.layers, currentLayer)
			: board.data?.layers[currentLayer]?.name
		: undefined;

	// Only where the board replaced the global sidebar — embedded hosts still
	// show theirs, and a second avatar beside it is chrome the board did not
	// remove. Memoised, or a fresh element every board render would defeat
	// `BoardActivityRail`'s memo on every canvas drag frame.
	const railFooter = useMemo(
		() =>
			ownsWindow ? (
				<BoardAccountItem
					onOpenSettings={() => router.push("/settings")}
					onOpenNotifications={() => router.push("/notifications")}
				/>
			) : undefined,
		[ownsWindow, router],
	);

	return (
		<>
			<BoardShell
				rail={
					<BoardActivityRail
						top={railItems}
						bottom={railBottomItems}
						footer={railFooter}
					/>
				}
				sidebar={
					!isMobile && shell.sidebar ? (
						<BoardPane
							title={SIDEBAR_TITLES[shell.sidebar] ?? ""}
							onClose={surfaceActions.closeSidebar}
							bodyClassName={
								shell.sidebar === "variables" || shell.sidebar === "search"
									? "overflow-hidden"
									: undefined
							}
						>
							{sidebarBody}
						</BoardPane>
					) : undefined
				}
				tabs={
					board.data &&
					(modules.length > 0 || typeof version === "undefined") ? (
						<FlowModuleTabs
							board={board.data}
							activeModuleId={currentModuleId}
							openFileIds={openFileIds}
							onSelect={handleSelectModule}
							onCloseFile={handleCloseFile}
							executeCommand={executeCommand}
							readOnly={typeof version !== "undefined"}
							reservedRoots={moduleReservedRoots}
							trailing={<BoardEditorActions actions={editorActions} />}
						/>
					) : undefined
				}
				breadcrumb={
					<BoardBreadcrumb
						fileLabel={
							currentModuleId
								? `${board.data?.layers?.[currentModuleId]?.name ?? ""}${MODULE_FILE_EXTENSION}`
								: MAIN_FILE_LABEL
						}
						layerPath={layerPath}
						layerNames={layerNames}
						onJumpToLayer={jumpToLayer}
					/>
				}
				script={isMobile ? undefined : scriptPane}
				panel={
					!isMobile && shell.panel ? (
						<BoardPanel
							tabs={[
								{
									id: "problems",
									label: t("problems", "Problems"),
									badge: problemNodes.length,
									badgeTone: "danger",
								},
								{ id: "runs", label: t("runs", "Runs") },
								{ id: "traces", label: t("logs", "Logs") },
								{
									id: "tests",
									label: t("tests", "Tests"),
									badge: boardTestsFailed,
									badgeTone: "danger",
								},
							]}
							active={shell.panel}
							onSelect={(tab) =>
								surfaceActions.openPanel(tab as typeof shell.panel & string)
							}
							onClose={surfaceActions.closePanel}
						>
							{panelBody}
						</BoardPanel>
					) : undefined
				}
				secondary={isMobile ? undefined : secondaryPane}
				secondaryWide={
					shell.secondary === "flowpilot" && copilotWorkspaceVisible
				}
				statusBar={
					<BoardStatusBar
						left={
							<>
								{ownsWindow && (
									<BoardStatusItem
										icon={<HouseIcon />}
										title={t("navigate", "Navigate")}
										popoverClassName="w-64 p-1"
										popover={
											<BoardNavMenu
												appHref={appHref}
												boardParent={boardParent}
												boardId={boardId}
												onNavigate={(href) => router.push(href)}
											/>
										}
									>
										{app.data?.name ?? t("home", "Home")}
									</BoardStatusItem>
								)}
								{board.data && (
									<BoardStatusItem
										icon={<NotebookPenIcon />}
										title={t("boardSettings", "Board settings")}
										popover={
											<BoardIdentityForm
												appId={appId}
												boardId={boardId}
												board={board.data}
											/>
										}
									>
										{board.data.name}
									</BoardStatusItem>
								)}
								{awareness && connectionStatus === "connected" && (
									<BoardStatusItem icon={<WifiIcon />} tone="accent">
										{t("live", "Live")}
									</BoardStatusItem>
								)}
								{awareness && connectionStatus === "reconnecting" && (
									<BoardStatusItem icon={<WifiIcon />} tone="warning">
										{t("reconnecting", "Reconnecting…")}
									</BoardStatusItem>
								)}
								{awareness && connectionStatus === "disconnected" && (
									<BoardStatusItem
										icon={<WifiOffIcon />}
										tone="danger"
										onClick={() => reconnect()}
									>
										{t("disconnected", "Disconnected")}
									</BoardStatusItem>
								)}
								{!awareness && (
									<BoardStatusItem icon={<WifiOffIcon />} tone="muted">
										{t("offline", "Offline")}
									</BoardStatusItem>
								)}
								<BoardSyncStatusPill
									appId={appId}
									boardId={boardId}
									onOpenRecovery={openSyncRecovery}
								/>
								<BoardActivityIndicator boardId={boardId} />
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
										peerScopes={peerScopes}
										onJoinScope={
											backend.boardState.getFlowScriptScoped &&
											typeof version === "undefined"
												? joinFlowScriptScope
												: undefined
										}
									/>
								)}
								{followingSub && (
									<BoardStatusItem
										icon={<Eye />}
										tone="accent"
										onClick={() => stopFollowing()}
									>
										{t("following", "Following")}
									</BoardStatusItem>
								)}
							</>
						}
						right={
							<>
								{layerBreadcrumb && (
									<BoardStatusItem icon={<GitBranchIcon />} tone="muted">
										{layerBreadcrumb}
									</BoardStatusItem>
								)}
								{board.data && (
									<BoardStatusItem
										icon={executionModeIcon(
											board.data.execution_mode ?? IExecutionMode.Hybrid,
										)}
										tone="muted"
										title={t("runSettings", "Run settings")}
										popoverAlign="end"
										popover={
											<BoardRuntimeForm
												appId={appId}
												boardId={boardId}
												board={board.data}
												isOffline={
													app.data?.visibility === IAppVisibility.Offline
												}
											/>
										}
									>
										{board.data.log_level}
									</BoardStatusItem>
								)}
								{board.data && (
									<BoardStatusItem
										icon={<TagIcon />}
										tone={version ? "warning" : "muted"}
										title={t("version", "Version")}
										popoverAlign="end"
										popover={
											<BoardReleaseForm
												appId={appId}
												boardId={boardId}
												board={board.data}
												version={version}
												selectVersion={setVersion}
											/>
										}
									>
										{version
											? `v${version.join(".")} · ${t("readonly", "- Read-Only")}`
											: `v${(board.data.version ?? [0, 0, 0]).join(".")} · ${board.data.stage}`}
									</BoardStatusItem>
								)}
								<BoardStatusItem
									icon={<TriangleAlertIcon />}
									tone={problemNodes.length > 0 ? "danger" : "muted"}
									onClick={() => surfaceActions.openPanel("problems")}
									title={t("problems", "Problems")}
								>
									{problemNodes.length}
								</BoardStatusItem>
								<BoardStatusItem
									icon={<PanelBottomIcon />}
									tone="muted"
									onClick={() => surfaceActions.togglePanel("runs")}
									title={t("runHistory", "Run History")}
								/>
							</>
						}
					/>
				}
				canvas={
					<>
						<FlowContextMenu
							board={board.data}
							droppedPin={droppedPin}
							currentLayerId={currentLayer}
							onCommentPlace={onCommentPlace}
							refs={board.data?.refs || {}}
							onClose={() => setDroppedPin(undefined)}
							nodes={catalog.data ?? []}
							selectionCount={selectedNodeIds.length}
							movableSelectionCount={selectedMovableIds.length}
							onEditSelectionAsFlowScript={
								backend.boardState.getFlowScriptScoped &&
								typeof version === "undefined"
									? openFlowScriptForSelection
									: undefined
							}
							onMoveSelectionToModule={
								typeof version === "undefined"
									? (target) => void moveSelectionToModule(target)
									: undefined
							}
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
								className={`w-full flex-1 min-h-0 relative select-none touch-none ${isOver && "border-green-400 border-2 z-10"}`}
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
											Math.hypot(tt.clientX - startX, tt.clientY - startY) > 10
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
										{insideModule
											? modulePathLabel(board.data?.layers, currentLayer)
											: board.data?.layers[currentLayer]?.name}
									</h2>
								)}
								{version && (
									<h3 className="absolute top-0 mr-2 mt-2 right-0 z-10 text-muted pointer-events-none select-none">
										{t("version", "Version")} {version[0]}.{version[1]}.
										{version[2]} {t("readonly", "- Read-Only")}
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
									insideLayer={Boolean(currentLayer) && !insideModule}
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
										<div className="flex items-center gap-2 rounded-md border bg-background px-3 py-2 shadow-floating">
											<span
												className="h-2 w-4 rounded-full"
												style={{
													backgroundColor: typeToColor(
														(active?.data?.current as IVariable).data_type,
													),
												}}
											/>
											<span className="font-mono text-sm font-medium">
												{(active?.data?.current as IVariable).name}
											</span>
										</div>
									) : null}
								</DragOverlay>
							</div>
						</FlowContextMenu>
					</>
				}
				overlays={
					<>
						{(templateSelectorOpen || (isBoardEmpty && !currentLayer)) && (
							<FlowTemplateSelector
								onSelectTemplate={handleApplyTemplate}
								onDismiss={() => setTemplateSelectorOpen(false)}
							/>
						)}
						{chatOpen && awareness && (
							<div className="absolute bottom-2 right-3 z-50">
								<FlowChat
									messages={chatMessages}
									onSendMessage={sendMessage}
									onClose={() => setChatOpen(false)}
									peerUsers={peerUsers}
									sub={sub}
								/>
							</div>
						)}
						{renderOverlay?.()}
					</>
				}
			/>

			<BoardMobileHost
				open={isMobile && Boolean(shell.mobile)}
				title={
					shell.mobile
						? (MOBILE_TITLES[shell.mobile] ?? t("board", "Board"))
						: ""
				}
				onClose={surfaceActions.closeMobile}
				full={shell.mobile === "flowpilot" || shell.mobile === "script"}
			>
				{shell.mobile === "script"
					? scriptPane
					: shell.mobile === "inspector" || shell.mobile === "flowpilot"
						? secondaryPane
						: shell.mobile === "problems" ||
								shell.mobile === "runs" ||
								shell.mobile === "traces" ||
								shell.mobile === "tests"
							? panelBody
							: sidebarBody}
			</BoardMobileHost>

			<BoardSyncRecoveryDialog
				appId={appId}
				boardId={boardId}
				open={syncRecoveryOpen}
				onOpenChange={setSyncRecoveryOpen}
			/>
			<PinEditModal appId={appId} boardId={boardId} version={version} />
			<FlowNodeInfoOverlay
				key={boardId}
				ref={nodeInfoOverlayRef}
				refs={board.data?.refs}
				boardRef={boardRef}
				onFocusNode={focusNode}
			/>
			<FlowSearch
				board={board.data}
				open={searchOpen}
				onOpenChange={setSearchOpen}
				onNavigate={focusNode}
				mode="dialog"
				onSwitchToSidebar={() => {
					setSearchOpen(false);
					surfaceActions.openSidebar("search");
				}}
			/>

			{/* Runtime Variables Prompt */}
			<RuntimeVariablesPrompt
				open={runtimeVarsPromptOpen}
				onOpenChange={(open) => {
					// ESC / X / overlay dismissal must settle a pending resume promise.
					if (open) setRuntimeVarsPromptOpen(true);
					else handleRuntimeVarsCancel();
				}}
				variables={runtimeConfiguredVars}
				existingValues={existingRuntimeVars}
				onSave={handleRuntimeVarsSave}
				onCancel={handleRuntimeVarsCancel}
				refs={board.data?.refs}
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
				selectionCount={selectedNodeIds.length}
			/>

			{/* Event payload for FlowScript lens runs — same form the canvas play button opens */}
			<Dialog
				open={typeof runDialogNode !== "undefined"}
				onOpenChange={(open) => {
					if (!open) closeRunDialog();
				}}
			>
				<DialogContent className="max-w-lg">
					<DialogHeader>
						<DialogTitle>
							{t("common:executeFriendly_name", "Execute {{friendly_name}}", {
								friendly_name: runDialogNode?.friendly_name,
							})}
						</DialogTitle>
						<DialogDescription>
							{t(
								"common:provideInputValuesForTheEventPayload",
								"Provide input values for the event payload.",
							)}
						</DialogDescription>
					</DialogHeader>
					{runDialogNode && (
						<EventPayloadForm
							node={runDialogNode}
							boardRef={boardRef}
							onLocalExecute={
								runDialogCapability?.local ? runDialogLocalExecute : undefined
							}
							onRemoteExecute={
								runDialogCapability?.remote ? runDialogRemoteExecute : undefined
							}
							canLocalExecute={runDialogCapability?.local ?? false}
							canRemoteExecute={runDialogCapability?.remote ?? false}
							onClose={closeRunDialog}
						/>
					)}
				</DialogContent>
			</Dialog>
		</>
	);
}
