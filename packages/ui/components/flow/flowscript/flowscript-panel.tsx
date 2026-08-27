"use client";

import { useTranslation } from "@flow-like/locales";
import Editor, { type Monaco, type OnMount } from "@monaco-editor/react";
import { createId } from "@paralleldrive/cuid2";
import {
	AlertTriangleIcon,
	AnchorIcon,
	BugIcon,
	CopyIcon,
	FileCode2Icon,
	FocusIcon,
	Loader2Icon,
	Maximize2Icon,
	Minimize2Icon,
	RefreshCcwIcon,
	Undo2Icon,
	WandSparklesIcon,
	XIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import {
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
	useSyncExternalStore,
} from "react";
import { toast } from "sonner";
import { useDeveloperMode } from "../../../hooks/use-developer-mode";
import {
	PEER_COLOR_COUNT,
	type PeerUserInfo,
	peerColorSlot,
	usePeerUserInfo,
} from "../../../hooks/use-peer-users";
import { formatRelativeTime } from "../../../lib/date";
import {
	type IBoardModule,
	MAIN_FILE_ID,
	fileModuleId,
} from "../../../lib/flow-modules";
import {
	getFlowScriptNamesTable,
	onFlowScriptNamesTableLoaded,
} from "../../../lib/flowscript/names";
import {
	FLOW_KEY_OPT_OUT_CLASS,
	shieldFlowBoardKeys,
} from "../../../lib/monaco-key-guard";
import type { IComment } from "../../../lib/schema/flow/board";
import type { INode } from "../../../lib/schema/flow/node";
import { useBackend } from "../../../state/backend-state";
import type {
	IApplyFlowScriptResponse,
	ICheckFlowScriptReconcileResponse,
	IFlowScriptDiagnostic,
} from "../../../state/backend-state/board-state";
import { useSuppressFabBubble } from "../../../state/fab-bubble";
import { useLogAggregation } from "../../../state/log-aggregation-state";
import { useRunExecutionStore } from "../../../state/run-execution-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Badge,
	Button,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "../../ui";
import {
	anchorAtLine,
	anchorAtOrAbove,
	parseFlowScriptAnchors,
} from "./flowscript-anchors";
import {
	type DestructiveCommandSummary,
	FlowScriptApplyPreviewChip,
	FlowScriptApplyPreviewList,
	destructiveCommandSummaries,
	summarizeBoardCommands,
} from "./flowscript-apply-preview";
import {
	FlowScriptCommentOverlay,
	type FlowScriptCommentThreadState,
} from "./flowscript-comment-widgets";
import {
	type FlowScriptNodeSpatial,
	buildFlowScriptComment,
	deriveFlowScriptCommentAddLines,
	deriveFlowScriptCommentIndicators,
	deriveFlowScriptCommentThreads,
	formatFlowScriptCommentPreview,
	withFlowScriptCommentContent,
} from "./flowscript-comments";
import { FlowScriptFileTabs, flowScriptFileTabs } from "./flowscript-file-tabs";
import {
	FLOWSCRIPT_DIAGNOSTIC_OWNER,
	FLOWSCRIPT_LANGUAGE_ID,
	FLOWSCRIPT_THEME_DARK,
	FLOWSCRIPT_THEME_LIGHT,
	type FlowScriptBoardScope,
	defineFlowScriptThemes,
	registerFlowScriptLanguage,
} from "./flowscript-language";
import { registerFlowScriptProviders } from "./flowscript-language-providers";
import {
	type FlowScriptConflictLensHandle,
	type FlowScriptConflictLensLabels,
	type FlowScriptConflictResolution,
	type FlowScriptMergeConflict,
	intersectRemoteTouched,
	mergeFlowScript,
	registerFlowScriptConflictLens,
	resolveFlowScriptConflict,
} from "./flowscript-merge";
import {
	canApplyFlowScript,
	resolveFlowScriptScope,
	shouldReloadFlowScriptAfterApply,
} from "./flowscript-panel-state";
import {
	EMPTY_FLOWSCRIPT_PRESENCE,
	deriveClaimedAnchorIds,
	findClaimCollision,
	peersSharingFlowScriptScope,
	resolveWireCursor,
	resolveWireViewport,
	useFlowScriptPresence,
	useFlowScriptScopeBroadcast,
	useFlowScriptViewBroadcast,
	useFlowScriptViewportBroadcast,
} from "./flowscript-presence";
import { FlowScriptPresenceDebug } from "./flowscript-presence-debug";
import {
	type FlowScriptDeferredReloadRunner,
	type FlowScriptSeat,
	captureFlowScriptSeat,
	createDeferredReloadRunner,
	isMonacoWidgetOpen,
	resolveFlowScriptSeat,
	shouldDeferFlowScriptReload,
} from "./flowscript-rerender";
import {
	type FlowScriptRunCapability,
	type FlowScriptRunLensHandle,
	type FlowScriptRunLensLabels,
	type FlowScriptRunMode,
	registerFlowScriptRunLens,
} from "./flowscript-run-lens";
import {
	type FlowScriptRemoteExecutionLike,
	createCoalescedInvoker,
	deriveRunStatsInlays,
	deriveRunTraceLines,
	runStatsKey,
} from "./flowscript-run-trace";
import { computeFlowScriptMarkersPreferWorker } from "./flowscript-worker-client";
import type { FlowScriptFileStore } from "./use-flowscript-files";

const DESTRUCTIVE_BLOCK_PREFIX = "FlowScript edit would delete ";

const LINT_DEBOUNCE_MS = 300;
const RUN_TRACE_DEBOUNCE_MS = 100;
const CURSOR_SYNC_DEBOUNCE_MS = 150;
/** How long after a canvas-driven editor reveal cursor sync stays muted (loop guard). */
const CANVAS_SYNC_MUTE_MS = 250;
const LINE_FLASH_MS = 1300;
const DIM_ANCHORS_STORAGE_KEY = "flowscript-dim-anchors";

const IN_SYNC_CHECK_RESPONSE: ICheckFlowScriptReconcileResponse = {
	parse_valid: true,
	reconcile_valid: true,
	idempotent: true,
	command_count: 0,
	corrections: [],
	diagnostics: [],
	board_commands: [],
};

interface ApplyOptions {
	allowDeletions?: boolean;
	suppressBlockedToast?: boolean;
	/** Present when the panel is editing a selection-scoped or per-file render. */
	scopeAnchors?: string[];
	/**
	 * The file this text is: `"main"` or a module layer id. Only set in file mode — the host
	 * derives both the apply's `currentLayer` and its `module` identity from it.
	 */
	file?: string;
}

interface FlowScriptCheckState {
	/** The exact text the response was computed for — stale responses are ignored. */
	forText: string;
	response: ICheckFlowScriptReconcileResponse;
}

type FlowScriptEditor = Parameters<OnMount>[0];
type DecorationsCollection = ReturnType<
	FlowScriptEditor["createDecorationsCollection"]
>;

const getEmptyPresenceSnapshot = () => EMPTY_FLOWSCRIPT_PRESENCE;

const EMPTY_DIRTY_FILES: ReadonlySet<string> = new Set();

export interface FlowScriptPanelProps {
	appId: string;
	boardId: string;
	/** Defined when viewing an old board version — the editor becomes read-only. */
	version?: [number, number, number];
	/** Bump to re-render the script when the board changed (e.g. react-query dataUpdatedAt). */
	boardUpdatedAt?: number;
	catalogNodes?: INode[];
	/** Node ids currently selected on the canvas (canvas → editor navigation). */
	selectedNodeIds?: string[];
	/** Transient canvas highlight for the entity under the text cursor; `undefined` clears it. */
	onHighlightNode?: (nodeId?: string) => void;
	/** The board's `focusNode` goto funnel — centers the viewport and opens the right layer. */
	onRevealNode?: (nodeId: string) => void;
	/** When set (and the backend supports it), the panel edits only these nodes' sections. */
	scopeNodeIds?: string[];
	/** Leave scoped mode and reload the whole board. */
	onExitScope?: () => void;
	/** The board's module layers, ordered by path — one file tab each, after `main.flow`. */
	modules?: readonly IBoardModule[];
	/**
	 * The file the canvas is on (`"main"` or a module layer id). There is exactly one current
	 * file per board: the canvas and every panel mount read it from here.
	 */
	currentFile?: string;
	/** Opens a file by opening its module on the canvas; `null` is `main`. */
	onSelectFile?: (moduleId: string | null) => void;
	/** Per-file buffers, owned by the board so both panel mounts share one stash. */
	files?: FlowScriptFileStore;
	/**
	 * The board's modules and their functions. Feeds the client linter so cross-file calls
	 * (`checkout::payments::helper()`, a root function called from a module file) are not
	 * reported as unknown — a file never declares what the other files hold.
	 */
	boardScope?: FlowScriptBoardScope;
	/** Total event/function sections on the board, for the scoped banner. */
	totalSections?: number;
	onApplyFlowScript: (
		flowscript: string,
		options?: ApplyOptions,
	) => Promise<IApplyFlowScriptResponse | undefined>;
	onClose: () => void;
	/** Yjs awareness of the board room — absent = single-user, no presence. */
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness?: any;
	/** The local user's sub — filters our own other sessions out of presence. */
	sub?: string;
	/** Peer identity cache (colors/names) shared with the canvas. */
	peerUsers?: Map<string, PeerUserInfo>;
	/** Follow mode: reveal + flash this node's line whenever `token` changes. */
	revealRequest?: { nodeId: string; token: number };
	/** Follow mode: keep this user's editor viewport at the top of ours while set. */
	followingSub?: string;
	/** Run an event from its header lens; absent = no run lenses at all. */
	onRunEventNode?: (nodeId: string, mode: FlowScriptRunMode) => void;
	/** Entry-node id → run modes the live board allows (gates the lenses). */
	runnableEventNodes?: ReadonlyMap<string, FlowScriptRunCapability>;
	/** Peers' currently executing nodes — line tints in the peer's color slot. */
	remoteExecutions?: readonly FlowScriptRemoteExecutionLike[];
	/** Board comments — text comments surface as margin threads on their anchor's line. */
	comments?: Record<string, IComment>;
	/** Create/update a board comment through the board's command funnel (undo-able). */
	onUpsertComment?: (comment: IComment) => Promise<void>;
	/** Delete a board comment through the board's command funnel (undo-able). */
	onRemoveComment?: (comment: IComment) => Promise<void>;
	/** Board-node position/layer lookup for placing editor-created comments on the canvas. */
	getNodeSpatial?: (nodeId: string) => FlowScriptNodeSpatial | undefined;
}

function rustDiagnosticToMarker(
	monaco: Monaco,
	text: string,
	diagnostic: IFlowScriptDiagnostic,
) {
	const lineText = text.split("\n")[diagnostic.line - 1] ?? "";
	// Underline the token at the error column (an identifier/number run) rather than the whole
	// remainder of the line; fall back to a single character on a symbol or at end-of-line.
	const token = /^[\w$]+/.exec(lineText.slice(diagnostic.col - 1))?.[0] ?? "";
	return {
		message: diagnostic.message,
		severity:
			diagnostic.severity === "error"
				? monaco.MarkerSeverity.Error
				: monaco.MarkerSeverity.Warning,
		startLineNumber: diagnostic.line,
		startColumn: diagnostic.col,
		endLineNumber: diagnostic.line,
		endColumn: diagnostic.col + Math.max(token.length, 1),
	};
}

const PRESENCE_DEBUG_STORAGE_KEY = "flowscript.presenceDebug";

function readDimAnchorsPreference(): boolean {
	try {
		return localStorage.getItem(DIM_ANCHORS_STORAGE_KEY) !== "off";
	} catch {
		return true;
	}
}

function writeDimAnchorsPreference(dim: boolean): void {
	try {
		localStorage.setItem(DIM_ANCHORS_STORAGE_KEY, dim ? "on" : "off");
	} catch {
		// Preference is a convenience; private mode etc. may block storage.
	}
}

export function FlowScriptPanel({
	appId,
	boardId,
	version,
	boardUpdatedAt,
	catalogNodes,
	selectedNodeIds,
	onHighlightNode,
	onRevealNode,
	scopeNodeIds,
	onExitScope,
	modules,
	currentFile,
	onSelectFile,
	files,
	boardScope,
	totalSections,
	onApplyFlowScript,
	onClose,
	awareness,
	sub,
	peerUsers,
	revealRequest,
	followingSub,
	onRunEventNode,
	runnableEventNodes,
	remoteExecutions,
	comments,
	onUpsertComment,
	onRemoveComment,
	getNodeSpatial,
}: Readonly<FlowScriptPanelProps>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const { resolvedTheme } = useTheme();
	// This panel's footer (Apply/Reset) sits exactly where the global FlowPilot bubble floats;
	// suppress the bubble while the panel is open so they don't overlap.
	useSuppressFabBubble();

	const [text, setText] = useState("");
	const [baseline, setBaseline] = useState("");
	const [loading, setLoading] = useState(true);
	const [loadError, setLoadError] = useState<string | undefined>(undefined);
	const [applying, setApplying] = useState(false);
	const [diagnostics, setDiagnostics] = useState<string[]>([]);
	const [boardChangedBehindEdits, setBoardChangedBehindEdits] = useState(false);
	const [refreshConfirmationOpen, setRefreshConfirmationOpen] = useState(false);
	const [editorReady, setEditorReady] = useState(false);
	const [namesReady, setNamesReady] = useState(
		() => getFlowScriptNamesTable() !== undefined,
	);
	useEffect(() => onFlowScriptNamesTableLoaded(() => setNamesReady(true)), []);
	const [destructiveMessage, setDestructiveMessage] = useState<
		string | undefined
	>(undefined);
	const [dimAnchors, setDimAnchors] = useState(readDimAnchorsPreference);
	// Developer mode only: a live readout of the presence pipeline.
	const { developerMode } = useDeveloperMode();
	const [presenceDebug, setPresenceDebug] = useState(() => {
		try {
			return localStorage.getItem(PRESENCE_DEBUG_STORAGE_KEY) === "on";
		} catch {
			return false;
		}
	});
	const togglePresenceDebug = useCallback(() => {
		setPresenceDebug((value) => {
			try {
				localStorage.setItem(PRESENCE_DEBUG_STORAGE_KEY, value ? "off" : "on");
			} catch {}
			return !value;
		});
	}, []);
	const editorHasTextFocus = useCallback(
		() => editorRef.current?.hasTextFocus() ?? false,
		[],
	);
	const [scopeAnchors, setScopeAnchors] = useState<string[] | undefined>(
		undefined,
	);
	const [checkState, setCheckState] = useState<
		FlowScriptCheckState | undefined
	>(undefined);
	const [checking, setChecking] = useState(false);
	const [previewExpanded, setPreviewExpanded] = useState(false);
	const [fullScreen, setFullScreen] = useState(false);
	const [pendingDeletions, setPendingDeletions] = useState<
		DestructiveCommandSummary[] | undefined
	>(undefined);
	const [mergeConflicts, setMergeConflicts] = useState<
		FlowScriptMergeConflict[]
	>([]);
	/** Anchors changed remotely since the local baseline (union over merges). */
	const [remoteTouched, setRemoteTouched] = useState<ReadonlySet<string>>(
		() => new Set(),
	);
	const [commentThreadState, setCommentThreadState] = useState<
		FlowScriptCommentThreadState | undefined
	>(undefined);

	const readOnly = typeof version !== "undefined";
	const dirty = text !== baseline;
	const applyState = {
		readOnly,
		dirty,
		applying,
		loading,
		boardChangedBehindEdits,
		unresolvedConflicts: mergeConflicts.length > 0,
	};
	const canApply = canApplyFlowScript(applyState);
	const applyStateRef = useRef(applyState);
	applyStateRef.current = applyState;

	const dirtyRef = useRef(dirty);
	dirtyRef.current = dirty;
	const textRef = useRef(text);
	textRef.current = text;
	const baselineRef = useRef(baseline);
	baselineRef.current = baseline;
	const loadErrorRef = useRef(loadError);
	loadErrorRef.current = loadError;
	// Mount-time Monaco registrations (format provider, actions) outlive any one
	// render; hand them the current translations through refs. Plain t() calls
	// here keep the keys visible to the i18n extractor.
	const formatFailedLabel = t(
		"flowscriptFormatFailed",
		"FlowScript format failed",
	);
	const formatFailedLabelRef = useRef(formatFailedLabel);
	formatFailedLabelRef.current = formatFailedLabel;
	const revealOnBoardLabel = t("revealOnBoard", "Reveal on board");
	const revealOnBoardLabelRef = useRef(revealOnBoardLabel);
	revealOnBoardLabelRef.current = revealOnBoardLabel;
	const commentsOnLineLabel = t(
		"flowscriptCommentsOnLine",
		"Comments on this line",
	);
	const commentsOnLineLabelRef = useRef(commentsOnLineLabel);
	commentsOnLineLabelRef.current = commentsOnLineLabel;
	const addCommentLabel = t("flowscriptAddComment", "Add comment");
	const addCommentLabelRef = useRef(addCommentLabel);
	addCommentLabelRef.current = addCommentLabel;
	const noCommentAnchorLabel = t(
		"flowscriptNoCommentAnchor",
		"No statement here to attach a comment to",
	);
	const noCommentAnchorLabelRef = useRef(noCommentAnchorLabel);
	noCommentAnchorLabelRef.current = noCommentAnchorLabel;
	const runLensLabels: FlowScriptRunLensLabels = {
		runEvent: t("runEvent", "Run"),
		runRemote: t("runRemote", "Run on server"),
		applyBeforeRun: t("applyBeforeRun", "Apply to board before running"),
	};
	const runLensLabelsRef = useRef(runLensLabels);
	runLensLabelsRef.current = runLensLabels;
	const conflictLensLabels: FlowScriptConflictLensLabels = {
		keepMine: t("flowscriptKeepMine", "Keep mine"),
		takeTheirs: t("flowscriptTakeTheirs", "Take theirs"),
	};
	const conflictLensLabelsRef = useRef(conflictLensLabels);
	conflictLensLabelsRef.current = conflictLensLabels;
	const mergeConflictsRef = useRef(mergeConflicts);
	mergeConflictsRef.current = mergeConflicts;
	const runLensGate = { readOnly, dirty, runnableNodes: runnableEventNodes };
	const runLensGateRef = useRef(runLensGate);
	runLensGateRef.current = runLensGate;
	const onRunEventNodeRef = useRef(onRunEventNode);
	onRunEventNodeRef.current = onRunEventNode;
	const remoteExecutionsRef = useRef(remoteExecutions);
	remoteExecutionsRef.current = remoteExecutions;
	const backendRef = useRef(backend);
	backendRef.current = backend;
	const appIdRef = useRef(appId);
	appIdRef.current = appId;
	const boardIdRef = useRef(boardId);
	boardIdRef.current = boardId;
	const scopeAnchorsRef = useRef(scopeAnchors);
	scopeAnchorsRef.current = scopeAnchors;
	const checkStateRef = useRef(checkState);
	checkStateRef.current = checkState;
	const onHighlightNodeRef = useRef(onHighlightNode);
	onHighlightNodeRef.current = onHighlightNode;
	const peerUsersRef = useRef(peerUsers);
	peerUsersRef.current = peerUsers;
	const presenceSnapshotRef = useRef(EMPTY_FLOWSCRIPT_PRESENCE);
	const onRevealNodeRef = useRef(onRevealNode);
	onRevealNodeRef.current = onRevealNode;
	const onExitScopeRef = useRef(onExitScope);
	onExitScopeRef.current = onExitScope;
	const onUpsertCommentRef = useRef(onUpsertComment);
	onUpsertCommentRef.current = onUpsertComment;
	const onRemoveCommentRef = useRef(onRemoveComment);
	onRemoveCommentRef.current = onRemoveComment;
	const getNodeSpatialRef = useRef(getNodeSpatial);
	getNodeSpatialRef.current = getNodeSpatial;
	const subRef = useRef(sub);
	subRef.current = sub;

	const catalogRef = useRef<INode[] | undefined>(catalogNodes);
	catalogRef.current = catalogNodes;
	const boardScopeRef = useRef(boardScope);
	boardScopeRef.current = boardScope;
	// The scope object is rebuilt on every board refetch; only its content decides whether the
	// document has to be re-linted.
	const boardScopeKey = useMemo(
		() => (boardScope ? JSON.stringify(boardScope) : ""),
		[boardScope],
	);
	const providersDisposable = useRef<{ dispose: () => void } | null>(null);
	const formatProviderDisposable = useRef<{ dispose: () => void } | null>(null);
	const runLensHandleRef = useRef<FlowScriptRunLensHandle | null>(null);
	const conflictLensHandleRef = useRef<FlowScriptConflictLensHandle | null>(
		null,
	);
	const editorRef = useRef<Parameters<OnMount>[0] | null>(null);
	const monacoRef = useRef<Monaco | null>(null);
	const containerRef = useRef<HTMLDivElement | null>(null);
	// Live re-render machinery: typing/composition state feeds the reload
	// guard; a captured seat is restored once the re-rendered text is in.
	const lastInputAtRef = useRef(0);
	const composingRef = useRef(false);
	const remoteRefreshRunnerRef = useRef<FlowScriptDeferredReloadRunner | null>(
		null,
	);
	const pendingSeatRef = useRef<
		| {
				seat: FlowScriptSeat;
				oldIndex: ReturnType<typeof parseFlowScriptAnchors>;
				/** The exact text the swap will install; set once known. A text
				 *  change that does not match is NOT the swap (the user typed, or
				 *  the reload was a no-op) — the seat is dropped, never replayed. */
				expectedText?: string;
		  }
		| undefined
	>(undefined);
	/** The dirty buffer as it was right before the last statement merge. */
	const preMergeLocalTextRef = useRef<string | undefined>(undefined);

	// Board ↔ text navigation state.
	const anchorIndex = useMemo(() => parseFlowScriptAnchors(text), [text]);
	const anchorIndexRef = useRef(anchorIndex);
	anchorIndexRef.current = anchorIndex;
	const anchorDecorationsRef = useRef<DecorationsCollection | null>(null);
	const flashDecorationsRef = useRef<DecorationsCollection | null>(null);
	const flashTimeoutRef = useRef<ReturnType<typeof setTimeout> | undefined>(
		undefined,
	);
	const cursorDebounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(
		undefined,
	);
	const canvasSyncAtRef = useRef(0);
	const lastCursorHighlightRef = useRef<string | undefined>(undefined);
	const exitScopeAfterConfirmRef = useRef(false);

	// The file the panel edits. One current file per board: it comes from the canvas, so opening
	// a tab here and walking into a module there are the same navigation.
	const fileId = currentFile ?? MAIN_FILE_ID;
	// Deliberately keyed on primitives, never on the `modules` array: it is rebuilt on every board
	// refetch, and a new scope identity would re-render the file and throw the draft away.
	const hasModules = (modules?.length ?? 0) > 0;
	const scopeMode = useMemo(
		() =>
			resolveFlowScriptScope(
				scopeNodeIds,
				Boolean(backend.boardState.getFlowScriptScoped),
				{
					hasModules,
					backendSupportsFiles: Boolean(backend.boardState.getFlowScriptFile),
					file: fileId,
				},
			),
		[scopeNodeIds, backend, hasModules, fileId],
	);
	const scoped = scopeMode.kind === "scoped" && !readOnly;
	/** The module identity an apply/check of the current buffer must carry; main sends none. */
	const applyModuleId =
		scopeMode.kind === "file" ? fileModuleId(scopeMode.file) : undefined;
	const applyFileRef = useRef<string | undefined>(undefined);
	applyFileRef.current = scopeMode.kind === "file" ? scopeMode.file : undefined;
	const applyModuleIdRef = useRef(applyModuleId);
	applyModuleIdRef.current = applyModuleId;
	const filesRef = useRef(files);
	filesRef.current = files;

	// Shared scoped sessions: broadcast this panel's scope node ids while it is
	// open in scoped mode so teammates can join from the presence bar. Persists
	// across editor blur; withdrawn on scope exit, panel close, and unmount.
	// Which file this editor shows, so the presence list can say "in main.flow".
	useFlowScriptViewBroadcast({ awareness, enabled: !readOnly, file: fileId });
	// The top of this editor's viewport, so a teammate can scroll-follow it.
	useFlowScriptViewportBroadcast({
		awareness,
		enabled: !readOnly,
		editor: editorReady ? editorRef.current : null,
		anchorIndexRef,
	});
	useFlowScriptScopeBroadcast({
		awareness,
		enabled: scoped,
		nodeIds: scopeMode.kind === "scoped" ? scopeMode.nodeIds : undefined,
	});

	// `version` is a fresh array reference every render; key on its stable string
	// form so load() (and the effects depending on it) don't re-fire in a loop.
	const versionKey = version?.join("_");
	// One render fetch for both the full reload and the dirty-buffer merge —
	// honors the active scope so a scoped render refreshes as a scoped render.
	const fetchBoardRender = useCallback(async (): Promise<{
		flowscript: string;
		scopeAnchors?: string[];
	}> => {
		const parsedVersion = versionKey
			? (versionKey.split("_").map(Number) as [number, number, number])
			: undefined;
		const getScoped = backend.boardState.getFlowScriptScoped?.bind(
			backend.boardState,
		);
		if (scopeMode.kind === "scoped" && !parsedVersion && getScoped) {
			const scopedScript = await getScoped(
				appId,
				boardId,
				scopeMode.nodeIds,
				true,
			);
			return {
				flowscript: scopedScript.flowscript,
				scopeAnchors: scopedScript.scope_anchors,
			};
		}
		const getFile = backend.boardState.getFlowScriptFile?.bind(
			backend.boardState,
		);
		if (scopeMode.kind === "file" && !parsedVersion && getFile) {
			const file = await getFile(appId, boardId, scopeMode.file, true);
			// The file's own anchors are what limits its apply — everything the file did not
			// render (other modules, the root's sections) stays outside the reconcile diff.
			return {
				flowscript: file.flowscript,
				scopeAnchors: file.scope_anchors,
			};
		}
		const script = await backend.boardState.getFlowScript(
			appId,
			boardId,
			parsedVersion,
			true,
		);
		return { flowscript: script };
	}, [backend, appId, boardId, versionKey, scopeMode]);

	// Every render request carries a token: a switch away (to another file, or to a stashed
	// buffer) invalidates the one in flight, so a slow response can never overwrite the document
	// the user is now looking at.
	const loadTokenRef = useRef(0);
	const load = useCallback(async (): Promise<string | undefined> => {
		const token = ++loadTokenRef.current;
		setLoading(true);
		setLoadError(undefined);
		try {
			const render = await fetchBoardRender();
			if (loadTokenRef.current !== token) return undefined;
			setText(render.flowscript);
			setBaseline(render.flowscript);
			setScopeAnchors(render.scopeAnchors);
			setBoardChangedBehindEdits(false);
			setMergeConflicts([]);
			setRemoteTouched(new Set());
			preMergeLocalTextRef.current = undefined;
			return render.flowscript;
		} catch (error) {
			if (loadTokenRef.current !== token) return undefined;
			setLoadError(
				error instanceof Error
					? error.message
					: t("failedToRenderFlowscript", "Failed to render FlowScript"),
			);
			return undefined;
		} finally {
			if (loadTokenRef.current === token) setLoading(false);
		}
	}, [fetchBoardRender, t]);

	/** Puts the editor back where the file was left; the model swap lands one frame later. */
	const restoreViewState = useCallback(
		(viewState: unknown, expectedText: string) => {
			const editor = editorRef.current;
			if (!editor || !viewState) return;
			const apply = () => {
				const model = editor.getModel();
				if (!model || model.getValue() !== expectedText) return false;
				// Programmatic move — mute editor→canvas cursor sync briefly.
				canvasSyncAtRef.current = Date.now();
				editor.restoreViewState(
					viewState as Parameters<typeof editor.restoreViewState>[0],
				);
				return true;
			};
			if (!apply()) {
				requestAnimationFrame(() => {
					apply();
				});
			}
		},
		[],
	);

	// Initial load, reload on board/version/scope switch, and file switches. Leaving a file keeps
	// its draft, baseline, anchors and editor seat in the board-owned stash; entering one restores
	// that buffer when it exists and re-renders from the board otherwise. Monaco's undo stack does
	// not survive the swap — the text does. Only file mode stashes: a whole-board or a
	// selection-scoped render is not any one file's buffer.
	const fileModeFile = scopeMode.kind === "file" ? scopeMode.file : undefined;
	const activeFileRef = useRef(fileModeFile);
	useEffect(() => {
		const previousFile = activeFileRef.current;
		if (previousFile === fileModeFile) {
			void load();
			return;
		}
		activeFileRef.current = fileModeFile;
		const store = filesRef.current;
		const editor = editorRef.current;
		if (
			previousFile !== undefined &&
			store &&
			!applyStateRef.current.loading &&
			!loadErrorRef.current &&
			baselineRef.current
		) {
			store.stash(previousFile, {
				text: textRef.current,
				baseline: baselineRef.current,
				scopeAnchors: scopeAnchorsRef.current,
				viewState: editor?.saveViewState() ?? undefined,
			});
		}
		// Conflicts, remote-touched anchors and the reconcile preview all belong to the file that
		// is leaving — they are re-derived for the incoming one.
		setMergeConflicts([]);
		setRemoteTouched(new Set());
		setBoardChangedBehindEdits(false);
		setCheckState(undefined);
		setDiagnostics([]);
		preMergeLocalTextRef.current = undefined;
		pendingSeatRef.current = undefined;

		const buffer =
			fileModeFile !== undefined ? store?.peek(fileModeFile) : undefined;
		if (!buffer) {
			void load();
			return;
		}
		// A render of the file we just left must not land on the buffer we restore here.
		loadTokenRef.current++;
		setLoading(false);
		setLoadError(undefined);
		setText(buffer.text);
		setBaseline(buffer.baseline);
		setScopeAnchors(buffer.scopeAnchors);
		restoreViewState(buffer.viewState, buffer.text);
	}, [load, fileModeFile, restoreViewState]);

	// Live re-render support: capture the cursor/selection/scroll as anchor-
	// relative offsets before a reload or merge swaps the buffer; the effect on
	// `text` below restores them against the NEW anchor index (rule 1: even a
	// clean re-render must not yank the cursor).
	const captureSeat = useCallback(() => {
		const editor = editorRef.current;
		const position = editor?.getPosition();
		if (!editor || !position) return;
		const selection = editor.getSelection();
		const selectionStart =
			selection &&
			(selection.selectionStartLineNumber !== position.lineNumber ||
				selection.selectionStartColumn !== position.column)
				? {
						lineNumber: selection.selectionStartLineNumber,
						column: selection.selectionStartColumn,
					}
				: undefined;
		const firstVisibleLine = editor.getVisibleRanges()[0]?.startLineNumber;
		const seat = captureFlowScriptSeat(anchorIndexRef.current, {
			position: { lineNumber: position.lineNumber, column: position.column },
			selectionStart,
			firstVisibleLine,
			firstVisibleLineOffsetPx:
				typeof firstVisibleLine === "number"
					? editor.getScrollTop() - editor.getTopForLineNumber(firstVisibleLine)
					: undefined,
		});
		pendingSeatRef.current = { seat, oldIndex: anchorIndexRef.current };
	}, []);

	// Restore the captured seat once the swapped-in text is in the model. Runs
	// on every text change but is a no-op without a pending seat.
	useEffect(() => {
		const pending = pendingSeatRef.current;
		if (!pending) return;
		if (pending.expectedText !== text) {
			// Once the expected swap text is known, any other change means the
			// swap never happened as captured — drop the seat instead of yanking
			// the cursor on a later keystroke. Before it is known, keep waiting.
			if (typeof pending.expectedText === "string")
				pendingSeatRef.current = undefined;
			return;
		}
		pendingSeatRef.current = undefined;
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;
		const applySeat = () => {
			const model = editor.getModel();
			if (!model || model.getValue() !== textRef.current) return false;
			const resolved = resolveFlowScriptSeat(
				pending.seat,
				pending.oldIndex,
				anchorIndexRef.current,
				model.getLineCount(),
			);
			const clampColumn = (line: number, column: number) =>
				Math.max(1, Math.min(column, model.getLineMaxColumn(line)));
			// Programmatic move — mute editor→canvas cursor sync briefly.
			canvasSyncAtRef.current = Date.now();
			const position = {
				lineNumber: resolved.position.lineNumber,
				column: clampColumn(
					resolved.position.lineNumber,
					resolved.position.column,
				),
			};
			if (resolved.selectionStart) {
				editor.setSelection(
					new monaco.Selection(
						resolved.selectionStart.lineNumber,
						clampColumn(
							resolved.selectionStart.lineNumber,
							resolved.selectionStart.column,
						),
						position.lineNumber,
						position.column,
					),
				);
			} else {
				editor.setPosition(position);
			}
			if (resolved.scroll) {
				editor.setScrollTop(
					Math.max(
						0,
						editor.getTopForLineNumber(resolved.scroll.lineNumber) +
							resolved.scroll.offsetPx,
					),
				);
			}
			return true;
		};
		// The Editor child applies the controlled value in its own effect (which
		// runs first); the rAF retry covers a late model swap.
		if (!applySeat()) {
			requestAnimationFrame(() => {
				applySeat();
			});
		}
	}, [text]);

	// Board mutated elsewhere (canvas edits, collaborators). CLEAN buffer: re-
	// render in place with the seat restored. DIRTY buffer: statement-level
	// three-way merge; only when the merge itself cannot run does the explicit
	// "board changed behind edits" guard remain. Both paths honor the scope.
	const applyRemoteBoardChange = useCallback(async () => {
		if (
			applyStateRef.current.readOnly ||
			applyStateRef.current.applying ||
			applyStateRef.current.loading
		)
			return;
		if (!dirtyRef.current) {
			captureSeat();
			const rendered = await load();
			const pending = pendingSeatRef.current;
			if (pending) {
				if (typeof rendered === "string") pending.expectedText = rendered;
				else pendingSeatRef.current = undefined;
			}
			return;
		}
		try {
			const fresh = await fetchBoardRender();
			if (fresh.flowscript === baselineRef.current) return;
			const result = mergeFlowScript({
				baseline: baselineRef.current,
				local: textRef.current,
				fresh: fresh.flowscript,
			});
			if (!result.ok) {
				setBoardChangedBehindEdits(true);
				return;
			}
			captureSeat();
			if (pendingSeatRef.current)
				pendingSeatRef.current.expectedText = result.mergedText;
			preMergeLocalTextRef.current = textRef.current;
			setText(result.mergedText);
			setBaseline(fresh.flowscript);
			setScopeAnchors(fresh.scopeAnchors);
			setBoardChangedBehindEdits(false);
			if (result.remoteTouchedAnchorIds.length > 0) {
				setRemoteTouched((previous) => {
					const next = new Set(previous);
					for (const id of result.remoteTouchedAnchorIds) next.add(id);
					return next;
				});
			}
			if (result.conflicts.length > 0) {
				// Earlier unresolved conflicts stay resolvable: their fresh blocks
				// are still the board's current text unless this merge re-conflicted
				// the same unit — then the new entry wins.
				setMergeConflicts((previous) => {
					const replaced = new Set(
						result.conflicts.map((conflict) => conflict.anchorId),
					);
					return [
						...previous.filter((conflict) => !replaced.has(conflict.anchorId)),
						...result.conflicts,
					];
				});
			} else {
				toast.info(
					t("flowscriptMergedRemote", {
						defaultValue_one:
							"Merged {{count}} remote change — your edits were kept",
						defaultValue_other:
							"Merged {{count}} remote changes — your edits were kept",
						count:
							result.stats.tookFresh + result.stats.freshAdded ||
							result.stats.tookLocal,
					}),
				);
			}
		} catch {
			setBoardChangedBehindEdits(true);
		}
	}, [captureSeat, load, fetchBoardRender, t]);

	// Deferral: never swap the buffer mid-typing (2s quiescence), during IME
	// composition, or while a suggest/find/rename widget is open. Deferred
	// changes re-check on a timer and flush on editor blur — never dropped.
	useEffect(() => {
		const runner = createDeferredReloadRunner({
			run: () => {
				void applyRemoteBoardChange();
			},
			isBlocked: () =>
				shouldDeferFlowScriptReload({
					now: Date.now(),
					editorFocused: editorRef.current?.hasTextFocus() ?? false,
					lastInputAt: lastInputAtRef.current || undefined,
					composing: composingRef.current,
					widgetOpen: isMonacoWidgetOpen(
						editorRef.current?.getContainerDomNode() ?? containerRef.current,
					),
				}),
		});
		remoteRefreshRunnerRef.current = runner;
		return () => {
			runner.dispose();
			remoteRefreshRunnerRef.current = null;
		};
	}, [applyRemoteBoardChange]);

	// The initial value is swallowed so mount doesn't double-fetch alongside
	// load(). Read-only/version-pinned panels never auto-reload.
	const lastBoardUpdateRef = useRef<number | undefined>(undefined);
	useEffect(() => {
		if (typeof boardUpdatedAt === "undefined") return;
		if (typeof lastBoardUpdateRef.current === "undefined") {
			lastBoardUpdateRef.current = boardUpdatedAt;
			return;
		}
		if (lastBoardUpdateRef.current === boardUpdatedAt) return;
		lastBoardUpdateRef.current = boardUpdatedAt;
		if (applyStateRef.current.readOnly) return;
		remoteRefreshRunnerRef.current?.request();
	}, [boardUpdatedAt]);

	useEffect(
		() => () => {
			providersDisposable.current?.dispose();
			providersDisposable.current = null;
			formatProviderDisposable.current?.dispose();
			formatProviderDisposable.current = null;
			runLensHandleRef.current?.dispose();
			runLensHandleRef.current = null;
			conflictLensHandleRef.current?.dispose();
			conflictLensHandleRef.current = null;
			if (flashTimeoutRef.current) clearTimeout(flashTimeoutRef.current);
			if (cursorDebounceRef.current) clearTimeout(cursorDebounceRef.current);
			onHighlightNodeRef.current?.(undefined);
		},
		[],
	);

	// The board's document-level shortcuts (delete selection, undo/redo, node
	// placement) must not fire while the caret is in Monaco; see monaco-key-guard.
	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;
		return shieldFlowBoardKeys(
			container,
			() => editorRef.current?.hasTextFocus() ?? false,
		);
	}, []);

	const runApply = useCallback(
		async (allowDeletions: boolean) => {
			if (!canApplyFlowScript(applyStateRef.current)) {
				if (applyStateRef.current.boardChangedBehindEdits) {
					toast.warning(
						t(
							"theBoardChangedWhileYouWereEditingRefreshFlowscriptBeforeApplyingYourDraft",
							"The board changed while you were editing. Refresh FlowScript before applying your draft.",
						),
					);
				} else if (applyStateRef.current.unresolvedConflicts) {
					toast.warning(
						t(
							"flowscriptResolveConflictsBeforeApply",
							"Resolve the merge conflicts before applying",
						),
					);
				}
				return;
			}
			applyStateRef.current = {
				...applyStateRef.current,
				applying: true,
			};
			setApplying(true);
			try {
				const result = await onApplyFlowScript(textRef.current, {
					allowDeletions,
					suppressBlockedToast: true,
					// The current file's anchors and its identity travel together: the host turns
					// `file` into the apply's `currentLayer` and `module`.
					scopeAnchors: scopeAnchorsRef.current,
					file: applyFileRef.current,
				});
				if (!result) return;

				const blocked =
					result.commands.length === 0 &&
					result.diagnostics[0]?.startsWith(DESTRUCTIVE_BLOCK_PREFIX);
				if (blocked) {
					setDestructiveMessage(result.diagnostics[0]);
					return;
				}

				setDiagnostics(result.diagnostics);
				if (
					shouldReloadFlowScriptAfterApply({
						commandCount: result.commands.length,
						correctionCount: result.corrections?.length ?? 0,
						diagnosticCount: result.diagnostics.length,
					})
				) {
					await load();
				}
			} catch {
				// applyFlowScript already surfaced the error via toast
			} finally {
				applyStateRef.current = {
					...applyStateRef.current,
					applying: false,
				};
				setApplying(false);
			}
		},
		[onApplyFlowScript, load, t],
	);

	// Advisory collision toast (collab rule 3): applying a draft that touches
	// statements a peer is currently editing proceeds (last-writer-wins) but
	// names the collision first. Claims come from the presence snapshot.
	const warnOnClaimCollision = useCallback(() => {
		const claims = presenceSnapshotRef.current.claims;
		if (claims.length === 0) return;
		const localIds = deriveClaimedAnchorIds(
			baselineRef.current,
			textRef.current,
		);
		if (localIds.length === 0) return;
		const hit = findClaimCollision(claims, new Set(localIds));
		if (!hit) return;
		const name =
			(hit.sub
				? peerUsersRef.current?.get(hit.sub)?.truncatedName
				: undefined) ?? t("common:user", "User");
		toast.warning(
			t("flowscriptEditCollision", {
				defaultValue: "This change touches statements {{name}} is editing",
				name,
			}),
		);
	}, [t]);

	// Apply entry point: when the current reconcile preview shows deletions, ask
	// first (naming what gets deleted) instead of round-tripping into the server-side
	// destructive block. A stale/absent preview falls through to that block.
	const requestApply = useCallback(() => {
		warnOnClaimCollision();
		const state = checkStateRef.current;
		if (state && state.forText === textRef.current) {
			const destructive = destructiveCommandSummaries(
				state.response.board_commands ?? [],
			);
			if (destructive.length > 0) {
				setPendingDeletions(destructive);
				return;
			}
		}
		void runApply(false);
	}, [runApply, warnOnClaimCollision]);

	const applyRef = useRef<() => void>(() => {});
	applyRef.current = requestApply;

	// Merge-conflict resolution: each conflict resolves per unit ("mine" keeps
	// the buffer's block, "theirs" splices the board's block); resolving the
	// last one simply leaves the buffer as a normal dirty draft over the fresh
	// baseline, so apply diffs exactly the user's decisions.
	const resolveConflict = useCallback(
		(conflictIndex: number, resolution: FlowScriptConflictResolution) => {
			const conflicts = mergeConflictsRef.current;
			const conflict = conflicts[conflictIndex];
			if (!conflict) return;
			const nextText = resolveFlowScriptConflict(
				textRef.current,
				conflict,
				resolution,
			);
			if (nextText !== textRef.current) setText(nextText);
			setMergeConflicts(
				conflicts.filter((_, index) => index !== conflictIndex),
			);
		},
		[],
	);

	const resolveAllConflicts = useCallback(
		(resolution: FlowScriptConflictResolution) => {
			let nextText = textRef.current;
			for (const conflict of mergeConflictsRef.current) {
				nextText = resolveFlowScriptConflict(nextText, conflict, resolution);
			}
			if (nextText !== textRef.current) setText(nextText);
			setMergeConflicts([]);
		},
		[],
	);

	// Clipboard escape hatch: the dirty buffer exactly as it was before the
	// last merge, in case the user wants their version back wholesale.
	const copyPreMergeVersion = useCallback(async () => {
		await navigator.clipboard.writeText(
			preMergeLocalTextRef.current ?? textRef.current,
		);
		toast.success(
			t("flowscriptCopiedToClipboard", "FlowScript copied to clipboard"),
		);
	}, [t]);

	const revealCursorLineOnBoard = useCallback(() => {
		const editor = editorRef.current;
		const line = editor?.getPosition()?.lineNumber;
		if (!line) return;
		const anchor =
			anchorAtLine(anchorIndexRef.current, line) ??
			anchorAtOrAbove(anchorIndexRef.current, line);
		if (!anchor || anchor.kind === "variable") return;
		onRevealNodeRef.current?.(anchor.id);
	}, []);

	// ── Board comments in the editor ─────────────────────────────────────
	// Text comments bound to a statement (Comment.node_id) surface as margin
	// threads on their anchor's line; every mutation routes through the board's
	// command funnel (undo-able, sync-propagated). Image/Video comments and
	// dangling/unanchored notes stay canvas-only.
	const commentsEnabled = typeof comments !== "undefined";
	const commentsEditable =
		commentsEnabled && !readOnly && Boolean(onUpsertComment);
	const commentsEnabledRef = useRef(commentsEnabled);
	commentsEnabledRef.current = commentsEnabled;
	const commentModel = useMemo(
		() => deriveFlowScriptCommentThreads(comments, anchorIndex),
		[comments, anchorIndex],
	);
	const commentModelRef = useRef(commentModel);
	commentModelRef.current = commentModel;
	const lookupUser = useMemo(
		() => backend.userState.lookupUser.bind(backend.userState),
		[backend],
	);
	const commentAuthorSubs = useMemo(() => {
		const subs = new Set<string>();
		for (const thread of commentModel.threads) {
			for (const comment of thread.comments) {
				if (comment.author && comment.author !== "anonymous")
					subs.add(comment.author);
			}
		}
		return [...subs];
	}, [commentModel]);
	const commentAuthors = usePeerUserInfo(commentAuthorSubs, lookupUser, 24);

	/** Every entry point (margin click, context menu, add affordance) lands here. */
	const openCommentsAtLine = useCallback(
		(line: number, focusComposer: boolean) => {
			if (!commentsEnabledRef.current) return;
			const model = commentModelRef.current;
			const anchor =
				anchorAtLine(anchorIndexRef.current, line) ??
				anchorAtOrAbove(anchorIndexRef.current, line);
			const thread =
				model.threads.find((entry) => entry.line === line) ??
				(anchor ? model.threadsByAnchorId.get(anchor.id) : undefined);
			if (thread) {
				setCommentThreadState({
					anchorId: thread.anchorId,
					line: thread.line,
					focusComposer,
				});
				return;
			}
			// Fresh threads bind to node statements only — variables and function
			// headers have no board node a comment could follow on the canvas.
			if (!anchor || anchor.kind !== "node") {
				toast.info(noCommentAnchorLabelRef.current);
				return;
			}
			setCommentThreadState({
				anchorId: anchor.id,
				line: anchor.line,
				focusComposer,
			});
		},
		[],
	);
	const openCommentsAtLineRef = useRef(openCommentsAtLine);
	openCommentsAtLineRef.current = openCommentsAtLine;
	const closeCommentThread = useCallback(
		() => setCommentThreadState(undefined),
		[],
	);

	const handleCreateComment = useCallback(
		async (anchorId: string, content: string) => {
			const upsert = onUpsertCommentRef.current;
			if (!upsert) return;
			await upsert(
				buildFlowScriptComment({
					id: createId(),
					anchorId,
					content,
					author: subRef.current,
					node: getNodeSpatialRef.current?.(anchorId),
					nowMs: Date.now(),
				}),
			);
		},
		[],
	);
	const handleUpdateComment = useCallback(
		async (comment: IComment, content: string) => {
			await onUpsertCommentRef.current?.(
				withFlowScriptCommentContent(comment, content),
			);
		},
		[],
	);
	const handleDeleteComment = useCallback(async (comment: IComment) => {
		await onRemoveCommentRef.current?.(comment);
	}, []);

	// The open thread's anchor left the document (statement deleted, scope
	// switch, reload) — close instead of floating on a stale line.
	useEffect(() => {
		if (!commentThreadState) return;
		if (!anchorIndex.firstLineById.has(commentThreadState.anchorId))
			setCommentThreadState(undefined);
	}, [anchorIndex, commentThreadState]);

	// Set once every decoration ref below exists; a remounted <Editor> (load
	// error → Retry) gets fresh collections instead of ones bound to the
	// disposed instance, whose `.set()` silently drops everything.
	const resetDecorationCollectionsRef = useRef<() => void>(() => {});
	const handleEditorMount: OnMount = useCallback(
		(editor, monaco) => {
			const remount =
				editorRef.current !== null && editorRef.current !== editor;
			editorRef.current = editor;
			monacoRef.current = monaco;
			if (remount) {
				resetDecorationCollectionsRef.current();
				// Two commits, so every `editorReady`-gated effect runs its cleanup
				// against the old instance and re-attaches to the new one.
				setEditorReady(false);
				queueMicrotask(() => setEditorReady(true));
			}
			registerFlowScriptLanguage(monaco);
			defineFlowScriptThemes(monaco);
			monaco.editor.setTheme(
				document.documentElement.classList.contains("dark")
					? FLOWSCRIPT_THEME_DARK
					: FLOWSCRIPT_THEME_LIGHT,
			);
			providersDisposable.current?.dispose();
			providersDisposable.current = registerFlowScriptProviders(
				monaco,
				() => catalogRef.current,
			);
			formatProviderDisposable.current?.dispose();
			formatProviderDisposable.current = backendRef.current.boardState
				.formatFlowScript
				? monaco.languages.registerDocumentFormattingEditProvider(
						FLOWSCRIPT_LANGUAGE_ID,
						{
							provideDocumentFormattingEdits: async (model) => {
								const format =
									backendRef.current.boardState.formatFlowScript?.bind(
										backendRef.current.boardState,
									);
								if (!format) return [];
								const source = model.getValue();
								try {
									const formatted = await format(
										appIdRef.current,
										boardIdRef.current,
										source,
										true,
									);
									if (typeof formatted !== "string" || formatted === source)
										return [];
									return [
										{ range: model.getFullModelRange(), text: formatted },
									];
								} catch (error) {
									// A parse error must never clear the buffer — surface it and
									// return no edits so the source stays exactly as typed.
									toast.error(formatFailedLabelRef.current, {
										description:
											error instanceof Error ? error.message : String(error),
									});
									return [];
								}
							},
						},
					)
				: null;
			runLensHandleRef.current?.dispose();
			runLensHandleRef.current = onRunEventNodeRef.current
				? registerFlowScriptRunLens(monaco, {
						editor,
						getCatalogNodes: () => catalogRef.current,
						getGate: () => runLensGateRef.current,
						getLabels: () => runLensLabelsRef.current,
						onRun: (nodeId, mode) => onRunEventNodeRef.current?.(nodeId, mode),
					})
				: null;
			conflictLensHandleRef.current?.dispose();
			conflictLensHandleRef.current = registerFlowScriptConflictLens(monaco, {
				editor,
				getConflicts: () => mergeConflictsRef.current,
				getLabels: () => conflictLensLabelsRef.current,
				onResolve: (conflictIndex, resolution) =>
					resolveConflict(conflictIndex, resolution),
			});
			// Reload-guard inputs: last real input timestamp (typing quiescence)
			// and IME composition state; blur flushes a deferred re-render.
			editor.onKeyDown(() => {
				lastInputAtRef.current = Date.now();
			});
			editor.onDidChangeModelContent(() => {
				if (editor.hasTextFocus()) lastInputAtRef.current = Date.now();
			});
			editor.onDidCompositionStart(() => {
				composingRef.current = true;
			});
			editor.onDidCompositionEnd(() => {
				composingRef.current = false;
			});
			editor.onDidBlurEditorText(() => {
				remoteRefreshRunnerRef.current?.poke();
			});
			editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
				applyRef.current();
			});
			editor.addAction({
				id: "flowscript.revealOnBoard",
				label: revealOnBoardLabelRef.current,
				keybindings: [
					monaco.KeyMod.CtrlCmd | monaco.KeyMod.Alt | monaco.KeyCode.KeyG,
				],
				contextMenuGroupId: "navigation",
				contextMenuOrder: 1.5,
				run: () => {
					revealCursorLineOnBoard();
				},
			});
			editor.addAction({
				id: "flowscript.commentsOnLine",
				label: commentsOnLineLabelRef.current,
				contextMenuGroupId: "navigation",
				contextMenuOrder: 1.6,
				run: () => {
					const line = editor.getPosition()?.lineNumber;
					if (line) openCommentsAtLineRef.current(line, false);
				},
			});
			editor.addAction({
				id: "flowscript.addComment",
				label: addCommentLabelRef.current,
				contextMenuGroupId: "navigation",
				contextMenuOrder: 1.7,
				run: () => {
					const line = editor.getPosition()?.lineNumber;
					if (line) openCommentsAtLineRef.current(line, true);
				},
			});
			// Margin clicks: a thread indicator opens its thread, the hover "+"
			// affordance opens the composer for the line's anchor.
			editor.onMouseDown((event) => {
				if (
					event.target.type !==
					monaco.editor.MouseTargetType.GUTTER_LINE_DECORATIONS
				)
					return;
				const line = event.target.position?.lineNumber;
				const element = event.target.element;
				if (!line || !element) return;
				if (element.classList.contains("flowscript-comment-margin"))
					openCommentsAtLineRef.current(line, false);
				else if (element.classList.contains("flowscript-comment-add"))
					openCommentsAtLineRef.current(line, true);
			});
			// Editor → canvas: highlight (never focus/center) the entity whose line
			// holds the cursor. Muted briefly after a canvas-driven reveal so the two
			// directions cannot feed each other.
			editor.onDidChangeCursorPosition((event) => {
				if (cursorDebounceRef.current) clearTimeout(cursorDebounceRef.current);
				cursorDebounceRef.current = setTimeout(() => {
					if (Date.now() - canvasSyncAtRef.current < CANVAS_SYNC_MUTE_MS)
						return;
					const anchor = anchorAtLine(
						anchorIndexRef.current,
						event.position.lineNumber,
					);
					const targetId =
						anchor && anchor.kind !== "variable" ? anchor.id : undefined;
					if (lastCursorHighlightRef.current === targetId) return;
					lastCursorHighlightRef.current = targetId;
					onHighlightNodeRef.current?.(targetId);
				}, CURSOR_SYNC_DEBOUNCE_MS);
			});
			setEditorReady(true);
		},
		[revealCursorLineOnBoard, resolveConflict],
	);

	// Dim every trailing anchor comment so the ids stop competing with the code.
	// Decorations only — the text (and thus apply fidelity) is never touched.
	const anchorDecorationKeyRef = useRef<string | undefined>(undefined);
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editorReady || !editor || !monaco) return;
		// Re-setting hundreds of decorations per keystroke is wasted work when the
		// anchors did not move (e.g. edits below the last anchored line).
		const decorationKey = dimAnchors
			? anchorIndex.anchors
					.map((anchor) => `${anchor.id}:${anchor.line}:${anchor.column}`)
					.join("|")
			: "off";
		if (anchorDecorationKeyRef.current === decorationKey) return;
		anchorDecorationKeyRef.current = decorationKey;
		const anchorDecorations =
			anchorDecorationsRef.current ?? editor.createDecorationsCollection([]);
		anchorDecorationsRef.current = anchorDecorations;
		anchorDecorations.set(
			dimAnchors
				? anchorIndex.anchors.map((anchor) => ({
						range: new monaco.Range(
							anchor.line,
							anchor.column,
							anchor.line,
							anchor.endColumn,
						),
						options: {
							inlineClassName: "flowscript-anchor-dim",
							inlineClassNameAffectsLetterSpacing: true,
						},
					}))
				: [],
		);
	}, [anchorIndex, dimAnchors, editorReady]);

	// Realtime presence: publish the local cursor/selection and dirty-buffer
	// claims (anchor-relative — rule 2: ids and offsets only, never text) and
	// subscribe to peers'. Degrades to a no-op store when awareness is absent.
	const { store: presenceStore } = useFlowScriptPresence({
		awareness,
		sub,
		enabled: !readOnly,
		editor: editorReady ? editorRef.current : null,
		anchorIndexRef,
		text,
		baseline,
	});
	const presenceSnapshot = useSyncExternalStore(
		presenceStore.subscribe,
		presenceStore.getSnapshot,
		getEmptyPresenceSnapshot,
	);
	presenceSnapshotRef.current = presenceSnapshot;

	// Peer presence decorations: remote carets + name flags, remote selections,
	// claim glyphs, and — the editor-side twin of the canvas selection ring —
	// a wash + gutter bar + name tag on the lines of nodes a teammate has
	// selected on the board, with a one-shot flash on the node they just
	// clicked. All positions resolve against THIS client's anchor index.
	const presenceDecorationsRef = useRef<DecorationsCollection | null>(null);
	const glyphMarginOnRef = useRef(false);
	// Peer click flashes live in their own collection: re-setting the main one
	// on every presence tick would re-create the line element and restart (or
	// cut short) the one-shot animation.
	const peerFlashDecorationsRef = useRef<DecorationsCollection | null>(null);
	const peerFlashKeyRef = useRef("");
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editorReady || !editor || !monaco) return;
		const model = editor.getModel();
		if (!model) return;
		const maxLine = model.getLineCount();
		const clampColumn = (line: number, column: number) =>
			Math.max(1, Math.min(column, model.getLineMaxColumn(line)));
		const peerName = (peer: { sub?: string; self?: boolean }) =>
			peer.self
				? t("you", "You")
				: ((peer.sub ? peerUsers?.get(peer.sub)?.truncatedName : undefined) ??
					t("common:user", "User"));
		const slotOf = (peerSub: string | undefined, clientId: number) =>
			peerColorSlot(peerSub) ?? clientId % PEER_COLOR_COUNT;
		type PresenceDecoration = Parameters<
			DecorationsCollection["set"]
		>[0][number];
		const decorations: PresenceDecoration[] = [];
		const stickiness =
			monaco.editor.TrackedRangeStickiness.NeverGrowsWhenTypingAtEdges;
		// Monaco renders injected text ONLY for decorations that span at least
		// one character — on a collapsed range the name tag silently vanishes
		// (verified against 0.54; a caret without its name is a 2px bar nobody
		// recognises). So a tag rides a one-character range next to its
		// column: `before` the character at the column, or `after` the last
		// character when the column is the end of the line.
		const injectTag = (
			line: number,
			column: number,
			content: string,
			inlineClassName: string,
		) => {
			const lineMax = model.getLineMaxColumn(line);
			if (lineMax <= 1) return;
			const injected = {
				content,
				inlineClassName,
				cursorStops: monaco.editor.InjectedTextCursorStops.None,
			};
			decorations.push(
				column < lineMax
					? {
							range: new monaco.Range(line, column, line, column + 1),
							options: { stickiness, before: injected },
						}
					: {
							range: new monaco.Range(line, lineMax - 1, line, lineMax),
							options: { stickiness, after: injected },
						},
			);
		};

		for (const remote of presenceSnapshot.cursors) {
			const resolved = resolveWireCursor(anchorIndex, remote.cursor, maxLine);
			if (!resolved) continue;
			const slot = slotOf(remote.sub, remote.clientId);
			const column = clampColumn(resolved.lineNumber, resolved.column);
			decorations.push({
				range: new monaco.Range(
					resolved.lineNumber,
					column,
					resolved.lineNumber,
					column,
				),
				options: {
					stickiness,
					beforeContentClassName: `flowscript-peer-caret flowscript-peer-slot-${slot}`,
				},
			});
			injectTag(
				resolved.lineNumber,
				column,
				peerName(remote),
				`flowscript-peer-flag flowscript-peer-slot-${slot}`,
			);
			if (resolved.selection) {
				decorations.push({
					range: new monaco.Range(
						resolved.selection.startLineNumber,
						clampColumn(
							resolved.selection.startLineNumber,
							resolved.selection.startColumn,
						),
						resolved.selection.endLineNumber,
						clampColumn(
							resolved.selection.endLineNumber,
							resolved.selection.endColumn,
						),
					),
					options: {
						className: `flowscript-peer-selection flowscript-peer-slot-${slot}`,
					},
				});
			}
		}

		let claimLines = 0;
		for (const claim of presenceSnapshot.claims) {
			const slot = slotOf(claim.sub, claim.clientId);
			const label = t("flowscriptBeingEditedBy", {
				defaultValue: "Being edited by {{name}}",
				name: peerName(claim),
			});
			for (const anchorId of claim.anchorIds) {
				const line = anchorIndex.firstLineById.get(anchorId);
				if (!line || line > maxLine) continue;
				claimLines++;
				decorations.push({
					range: new monaco.Range(line, 1, line, 1),
					options: {
						glyphMarginClassName: `flowscript-peer-claim-glyph flowscript-peer-slot-${slot}`,
						glyphMarginHoverMessage: { value: label },
					},
				});
			}
		}

		type PeerFlash = { key: string; line: number; slot: number };
		const flashes: PeerFlash[] = [];
		for (const selection of presenceSnapshot.canvasSelections) {
			const slot = slotOf(selection.sub, selection.clientId);
			let tagged = false;
			for (const nodeId of selection.nodeIds) {
				const line = anchorIndex.firstLineById.get(nodeId);
				if (!line || line > maxLine) continue;
				decorations.push({
					range: new monaco.Range(line, 1, line, 1),
					options: {
						isWholeLine: true,
						className: `flowscript-peer-canvas-line flowscript-peer-slot-${slot}`,
						linesDecorationsClassName: `flowscript-peer-canvas-gutter flowscript-peer-slot-${slot}`,
						linesDecorationsTooltip: t("flowscriptSelectedOnCanvasBy", {
							defaultValue: "{{name}} has this selected on the board",
							name: peerName(selection),
						}),
					},
				});
				if (tagged) continue;
				tagged = true;
				injectTag(
					line,
					model.getLineMaxColumn(line),
					`⌖ ${peerName(selection)}`,
					`flowscript-peer-canvas-flag flowscript-peer-slot-${slot}`,
				);
			}
			if (!selection.activeNodeId) continue;
			const line = anchorIndex.firstLineById.get(selection.activeNodeId);
			if (!line || line > maxLine) continue;
			flashes.push({
				key: `${selection.clientId}:${selection.activeNodeId}:${selection.activeNodeTs ?? 0}`,
				line,
				slot,
			});
		}
		const flashKey = flashes.map((flash) => flash.key).join("|");
		if (flashKey !== peerFlashKeyRef.current) {
			peerFlashKeyRef.current = flashKey;
			const peerFlashDecorations =
				peerFlashDecorationsRef.current ??
				editor.createDecorationsCollection([]);
			peerFlashDecorationsRef.current = peerFlashDecorations;
			peerFlashDecorations.set(
				flashes.map((flash) => ({
					range: new monaco.Range(flash.line, 1, flash.line, 1),
					options: {
						isWholeLine: true,
						className: `flowscript-peer-line-flash flowscript-peer-slot-${flash.slot}`,
					},
				})),
			);
		}

		const presenceDecorations =
			presenceDecorationsRef.current ?? editor.createDecorationsCollection([]);
		presenceDecorationsRef.current = presenceDecorations;
		presenceDecorations.set(decorations);

		// The glyph margin only exists while someone actually holds a claim.
		const wantGlyphMargin = claimLines > 0;
		if (glyphMarginOnRef.current !== wantGlyphMargin) {
			glyphMarginOnRef.current = wantGlyphMargin;
			editor.updateOptions({ glyphMargin: wantGlyphMargin });
		}
	}, [presenceSnapshot, anchorIndex, editorReady, peerUsers, t]);

	// Scroll-follow (presence bar "Follow"): keep the top of the followed user's
	// editor viewport at the top of ours. Their position arrives anchor-relative
	// and resolves against THIS buffer — an anchor we do not render (other file,
	// other scope) is nothing to do. The local user always wins: a wheel/drag
	// scroll or a keystroke pauses following briefly, and a timer catches up
	// once that pause lapses even if the peer sits still.
	const scrollFollowRef = useRef({
		seen: new Map<number, { key: string; changedAt: number }>(),
		userScrolledAt: 0,
		ownScrollUntil: 0,
	});
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!followingSub || !editorReady || !editor || !monaco) return;
		const FOLLOW_PAUSE_MS = 1500;
		// Monaco's smooth scroll (125ms) reports its own frames as scroll events.
		const OWN_SCROLL_GRACE_MS = 250;
		const follow = scrollFollowRef.current;
		const scrollListener = editor.onDidScrollChange((event) => {
			if (!event.scrollTopChanged || Date.now() < follow.ownScrollUntil) return;
			follow.userScrolledAt = Date.now();
		});

		// The followed user's most recently moved session leads (local clock —
		// peers' timestamps are never compared); ties go to the lowest clientId.
		const now = Date.now();
		const sessions = [...presenceSnapshot.viewports]
			.filter(([, remote]) => remote.sub === followingSub)
			.map(([clientId, remote]) => {
				const key = `${remote.viewport.anchor.id}:${remote.viewport.dLine}`;
				const seen = follow.seen.get(clientId);
				const changedAt = seen?.key === key ? seen.changedAt : now;
				follow.seen.set(clientId, { key, changedAt });
				return { clientId, viewport: remote.viewport, changedAt };
			});
		for (const clientId of follow.seen.keys()) {
			if (!sessions.some((session) => session.clientId === clientId))
				follow.seen.delete(clientId);
		}
		const target = sessions.reduce<(typeof sessions)[number] | undefined>(
			(best, session) =>
				!best || session.changedAt > best.changedAt ? session : best,
			undefined,
		);

		let timer: ReturnType<typeof setTimeout> | undefined;
		const apply = () => {
			timer = undefined;
			const model = editor.getModel();
			if (!target || !model) return;
			const line = resolveWireViewport(
				anchorIndex,
				target.viewport,
				model.getLineCount(),
			);
			if (typeof line === "undefined") return;
			const at = Date.now();
			const pausedFor = Math.max(
				follow.userScrolledAt + FOLLOW_PAUSE_MS - at,
				lastInputAtRef.current + FOLLOW_PAUSE_MS - at,
				composingRef.current ? FOLLOW_PAUSE_MS : 0,
			);
			if (pausedFor > 0) {
				timer = setTimeout(apply, pausedFor + 1);
				return;
			}
			const maxTop = Math.max(
				0,
				editor.getScrollHeight() - editor.getLayoutInfo().height,
			);
			const top = Math.min(editor.getTopForLineNumber(line), maxTop);
			if (Math.abs(editor.getScrollTop() - top) < 1) return;
			follow.ownScrollUntil = at + OWN_SCROLL_GRACE_MS;
			editor.setScrollTop(top, monaco.editor.ScrollType.Smooth);
		};
		apply();
		return () => {
			scrollListener.dispose();
			if (timer) clearTimeout(timer);
		};
	}, [followingSub, presenceSnapshot, anchorIndex, editorReady]);

	// Merge-conflict decorations: warning wash + gutter bar on each conflicted
	// unit's anchor line, and the "Keep mine / Take theirs" lens pair refreshed
	// whenever conflicts or the anchor lines move.
	const conflictDecorationsRef = useRef<DecorationsCollection | null>(null);
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editorReady || !editor || !monaco) return;
		const model = editor.getModel();
		if (!model) return;
		const maxLine = model.getLineCount();
		const collection =
			conflictDecorationsRef.current ?? editor.createDecorationsCollection([]);
		conflictDecorationsRef.current = collection;
		type ConflictDecoration = Parameters<
			DecorationsCollection["set"]
		>[0][number];
		const decorations: ConflictDecoration[] = [];
		for (const conflict of mergeConflicts) {
			const line = conflict.anchorId
				? anchorIndex.firstLineById.get(conflict.anchorId)
				: 1;
			if (!line || line > maxLine) continue;
			decorations.push({
				range: new monaco.Range(line, 1, line, 1),
				options: {
					isWholeLine: true,
					className: "flowscript-merge-conflict",
					linesDecorationsClassName: "flowscript-merge-conflict-gutter",
					hoverMessage: {
						value:
							conflict.kind === "remote-deleted"
								? t(
										"flowscriptConflictRemoteDeleted",
										"Deleted on the board while you edited it",
									)
								: t(
										"flowscriptConflictBothChanged",
										"Changed on the board and in your draft",
									),
					},
				},
			});
		}
		collection.set(decorations);
		conflictLensHandleRef.current?.refresh();
	}, [mergeConflicts, anchorIndex, editorReady, t]);

	// Comment indicators in the line-decorations margin: one peer-colored dot
	// (with count) per thread line, hover previews via the margin tooltip, and
	// a hover-revealed "+" on commentable lines without a thread. Written only
	// when the derived key actually moves (same contract as the run trace).
	const commentDecorationsRef = useRef<DecorationsCollection | null>(null);
	const commentDecorationKeyRef = useRef<string | undefined>(undefined);
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editorReady || !editor || !monaco) return;
		const model = editor.getModel();
		if (!model) return;
		const collection =
			commentDecorationsRef.current ?? editor.createDecorationsCollection([]);
		commentDecorationsRef.current = collection;
		if (!commentsEnabled) {
			if (commentDecorationKeyRef.current !== "") {
				commentDecorationKeyRef.current = "";
				collection.set([]);
			}
			return;
		}
		const nameFor = (author?: string) =>
			(author ? commentAuthors.get(author)?.name : undefined) ??
			t("common:user", "User");
		const timeFor = (ms: number) => formatRelativeTime(ms, "short");
		const { indicators, key: indicatorKey } = deriveFlowScriptCommentIndicators(
			commentModel.threads,
			peerColorSlot,
		);
		const previews = new Map(
			commentModel.threads.map((thread) => [
				thread.anchorId,
				formatFlowScriptCommentPreview(thread, nameFor, timeFor),
			]),
		);
		const addLines = commentsEditable
			? deriveFlowScriptCommentAddLines(anchorIndex, commentModel)
			: [];
		const key = `${indicatorKey}|p:${[...previews.values()].join("¦")}|a:${addLines.join(",")}`;
		if (commentDecorationKeyRef.current === key) return;
		commentDecorationKeyRef.current = key;
		const maxLine = model.getLineCount();
		type CommentDecoration = Parameters<
			DecorationsCollection["set"]
		>[0][number];
		const decorations: CommentDecoration[] = [];
		const threadLines = new Set<number>();
		for (const indicator of indicators) {
			if (indicator.line > maxLine) continue;
			threadLines.add(indicator.line);
			const slotClass =
				typeof indicator.slot === "number"
					? ` flowscript-peer-slot-${indicator.slot}`
					: "";
			const countClass =
				indicator.count >= 10
					? " flowscript-comment-count-many"
					: ` flowscript-comment-count-${indicator.count}`;
			decorations.push({
				range: new monaco.Range(indicator.line, 1, indicator.line, 1),
				options: {
					linesDecorationsClassName: `flowscript-comment-margin${countClass}${slotClass}`,
					linesDecorationsTooltip: previews.get(indicator.anchorId) ?? null,
				},
			});
		}
		for (const line of addLines) {
			if (line > maxLine || threadLines.has(line)) continue;
			decorations.push({
				range: new monaco.Range(line, 1, line, 1),
				options: {
					linesDecorationsClassName: "flowscript-comment-add",
					linesDecorationsTooltip: addCommentLabelRef.current,
				},
			});
		}
		collection.set(decorations);
	}, [
		commentModel,
		anchorIndex,
		commentAuthors,
		commentsEnabled,
		commentsEditable,
		editorReady,
		t,
	]);

	// CodeLens gate/label changes Monaco cannot observe (dirty flips, board
	// capability changes, language switches) — poke the provider to recompute.
	// biome-ignore lint/correctness/useExhaustiveDependencies: deps are re-render triggers — the provider reads current values through refs
	useEffect(() => {
		runLensHandleRef.current?.refresh();
	}, [
		dirty,
		readOnly,
		runnableEventNodes,
		editorReady,
		runLensLabels.runEvent,
		runLensLabels.runRemote,
		runLensLabels.applyBeforeRun,
	]);

	// Live run traces: map executing/finished node ids (plus peers' executing
	// sets) to anchored lines and tint them. One decorations collection, written
	// only when the derived line sets actually change (`key` comparison).
	const runTraceDecorationsRef = useRef<DecorationsCollection | null>(null);
	const runTraceKeyRef = useRef<string | undefined>(undefined);
	const applyRunTrace = useCallback(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;
		const model = editor.getModel();
		if (!model) return;
		const lines = deriveRunTraceLines({
			boardId: boardIdRef.current,
			runs: useRunExecutionStore.getState().runs,
			remoteExecutions: remoteExecutionsRef.current,
			firstLineById: anchorIndexRef.current.firstLineById,
			slotFor: peerColorSlot,
		});
		if (runTraceKeyRef.current === lines.key) return;
		runTraceKeyRef.current = lines.key;
		const traceCollection =
			runTraceDecorationsRef.current ?? editor.createDecorationsCollection([]);
		runTraceDecorationsRef.current = traceCollection;
		const maxLine = model.getLineCount();
		type TraceDecoration = Parameters<DecorationsCollection["set"]>[0][number];
		const wholeLine = (
			line: number,
			className: string,
			gutterClassName?: string,
		): TraceDecoration => ({
			range: new monaco.Range(line, 1, line, 1),
			options: {
				isWholeLine: true,
				className,
				linesDecorationsClassName: gutterClassName,
			},
		});
		const decorations: TraceDecoration[] = [];
		for (const line of lines.executing) {
			if (line > maxLine) continue;
			decorations.push(
				wholeLine(
					line,
					"flowscript-run-executing",
					"flowscript-run-executing-gutter",
				),
			);
		}
		for (const line of lines.done) {
			if (line > maxLine) continue;
			decorations.push(wholeLine(line, "flowscript-run-done"));
		}
		for (const entry of lines.remote) {
			if (entry.line > maxLine) continue;
			const slotClass =
				typeof entry.slot === "number"
					? ` flowscript-peer-slot-${entry.slot}`
					: "";
			decorations.push(
				wholeLine(
					entry.line,
					`flowscript-run-remote${slotClass}`,
					`flowscript-run-remote-gutter${slotClass}`,
				),
			);
		}
		traceCollection.set(decorations);
	}, []);

	// Store subscription stays imperative: a run streams hundreds of node events
	// and re-rendering this panel per event would be far too costly. Coalesced to
	// one decoration pass per RUN_TRACE_DEBOUNCE_MS window.
	useEffect(() => {
		if (!editorReady || readOnly) return;
		const invoker = createCoalescedInvoker(
			applyRunTrace,
			RUN_TRACE_DEBOUNCE_MS,
		);
		const unsubscribe = useRunExecutionStore.subscribe(() => invoker.trigger());
		applyRunTrace();
		return () => {
			unsubscribe();
			invoker.dispose();
			runTraceKeyRef.current = undefined;
			runTraceDecorationsRef.current?.set([]);
		};
	}, [editorReady, readOnly, applyRunTrace]);

	// Re-map immediately when the text moved the anchors or peers' sets changed.
	// biome-ignore lint/correctness/useExhaustiveDependencies: anchorIndex/remoteExecutions are re-map triggers read through refs
	useEffect(() => {
		if (!editorReady || readOnly) return;
		applyRunTrace();
	}, [anchorIndex, remoteExecutions, editorReady, readOnly, applyRunTrace]);

	// Post-run stats inlays (heatmap visits/errors per statement line), rendered
	// as subtle after-line text. Memoized on text version (anchorIndex identity)
	// + heatmap identity, so keystrokes and log ticks cost nothing here.
	const heatmapEnabled = useLogAggregation((state) => state.heatmapEnabled);
	const heatmap = useLogAggregation((state) => state.heatmap);
	const heatmapFilter = useLogAggregation((state) => state.filter);
	const statsInlays = useMemo(() => {
		if (!heatmapEnabled || !heatmap) return [];
		if (heatmapFilter?.appId !== appId || heatmapFilter?.boardId !== boardId)
			return [];
		return deriveRunStatsInlays(anchorIndex.anchors, heatmap.nodes);
	}, [heatmapEnabled, heatmap, heatmapFilter, anchorIndex, appId, boardId]);
	const statsDecorationsRef = useRef<DecorationsCollection | null>(null);
	resetDecorationCollectionsRef.current = () => {
		for (const ref of [
			anchorDecorationsRef,
			flashDecorationsRef,
			presenceDecorationsRef,
			peerFlashDecorationsRef,
			conflictDecorationsRef,
			commentDecorationsRef,
			runTraceDecorationsRef,
			statsDecorationsRef,
		])
			ref.current = null;
		anchorDecorationKeyRef.current = undefined;
		commentDecorationKeyRef.current = undefined;
		glyphMarginOnRef.current = false;
		peerFlashKeyRef.current = "";
	};
	const statsKeyRef = useRef<string | undefined>(undefined);
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editorReady || !editor || !monaco) return;
		const model = editor.getModel();
		if (!model) return;
		const key = runStatsKey(statsInlays);
		if (statsKeyRef.current === key) return;
		statsKeyRef.current = key;
		const statsCollection =
			statsDecorationsRef.current ?? editor.createDecorationsCollection([]);
		statsDecorationsRef.current = statsCollection;
		const maxLine = model.getLineCount();
		statsCollection.set(
			statsInlays
				.filter((inlay) => inlay.line <= maxLine)
				.map((inlay) => {
					const column = model.getLineMaxColumn(inlay.line);
					return {
						range: new monaco.Range(inlay.line, column, inlay.line, column),
						options: {
							stickiness:
								monaco.editor.TrackedRangeStickiness
									.NeverGrowsWhenTypingAtEdges,
							after: {
								content: inlay.text,
								inlineClassName: "flowscript-run-stats",
								cursorStops: monaco.editor.InjectedTextCursorStops.None,
							},
						},
					};
				}),
		);
	}, [statsInlays, editorReady]);

	// Scroll a line into view and flash it, muting editor→canvas cursor sync so
	// the two directions cannot feed each other (canvas selection and follow mode).
	const revealAndFlashLine = useCallback((line: number) => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;
		canvasSyncAtRef.current = Date.now();
		editor.revealLineInCenterIfOutsideViewport(line);
		const flashDecorations =
			flashDecorationsRef.current ?? editor.createDecorationsCollection([]);
		flashDecorationsRef.current = flashDecorations;
		flashDecorations.set([
			{
				range: new monaco.Range(line, 1, line, 1),
				options: { isWholeLine: true, className: "flowscript-line-flash" },
			},
		]);
		if (flashTimeoutRef.current) clearTimeout(flashTimeoutRef.current);
		flashTimeoutRef.current = setTimeout(() => {
			flashDecorationsRef.current?.set([]);
		}, LINE_FLASH_MS);
	}, []);

	// Canvas → editor: scroll to the (last) selected node's line and flash it.
	const selectedKey = selectedNodeIds?.join(",") ?? "";
	useEffect(() => {
		if (!editorReady) return;
		const ids = selectedKey ? selectedKey.split(",") : [];
		if (ids.length === 0) return;
		const line = anchorIndexRef.current.firstLineById.get(ids[ids.length - 1]);
		if (!line) return;
		revealAndFlashLine(line);
	}, [selectedKey, editorReady, revealAndFlashLine]);

	// Follow mode / canvas "Go to code" → editor: jump to the anchor's line.
	// The anchor may not exist yet when the reveal opened the panel (text still
	// loading) — retry as the index fills in, consuming each token exactly once.
	const consumedRevealTokenRef = useRef<number | undefined>(undefined);
	useEffect(() => {
		if (!revealRequest || !editorReady) return;
		if (consumedRevealTokenRef.current === revealRequest.token) return;
		const line = anchorIndex.firstLineById.get(revealRequest.nodeId);
		if (!line) {
			// Still loading: try again once the index fills in. Loaded and absent
			// (the peer is in another file, or a scoped render): the request must
			// not lie in wait to scroll a line the peer left long ago — hand it to
			// the canvas instead.
			if (loading) return;
			consumedRevealTokenRef.current = revealRequest.token;
			onRevealNode?.(revealRequest.nodeId);
			return;
		}
		consumedRevealTokenRef.current = revealRequest.token;
		revealAndFlashLine(line);
	}, [
		revealRequest,
		editorReady,
		anchorIndex,
		revealAndFlashLine,
		loading,
		onRevealNode,
	]);

	// Realtime linting: instant client-side structural markers everywhere, authoritative
	// positioned diagnostics from the native parser where available, and — on the same
	// debounce tick — the reconcile dry-run that powers the apply preview chip.
	// biome-ignore lint/correctness/useExhaustiveDependencies: editorReady gates the first run once the editor mounts; namesReady re-lints once the names snapshot arrives; scopeAnchors and the board scope are read from refs (boardScopeKey re-lints when the modules actually changed)
	useEffect(() => {
		const monaco = monacoRef.current;
		const editor = editorRef.current;
		if (!monaco || !editor) return;
		const model = editor.getModel();
		if (!model) return;
		const source = text;
		const handle = setTimeout(async () => {
			// Computed in the language worker for large documents (in-thread otherwise),
			// so a keystroke burst never pays for linting on the UI thread.
			const clientMarkers = await Promise.resolve(
				computeFlowScriptMarkersPreferWorker(
					monaco,
					model,
					catalogRef.current,
					boardScopeRef.current,
				),
			);
			let nativeMarkers: unknown[] = [];
			try {
				const lintDiagnostics =
					await backend.boardState.lintFlowScript?.(source);
				if (lintDiagnostics) {
					nativeMarkers = lintDiagnostics.map((diagnostic) =>
						rustDiagnosticToMarker(monaco, source, diagnostic),
					);
				}
			} catch {
				// Linting transport is best-effort; ignore failures.
			}
			// Bail if the model was swapped, or the text moved on while we awaited the
			// native lint — otherwise we'd paint stale markers at now-wrong positions.
			if (editor.getModel() !== model || model.getValue() !== source) return;
			monaco.editor.setModelMarkers(model, FLOWSCRIPT_DIAGNOSTIC_OWNER, [
				...clientMarkers,
				...nativeMarkers,
			] as Parameters<typeof monaco.editor.setModelMarkers>[2]);

			const check = backend.boardState.checkFlowScriptReconcile?.bind(
				backend.boardState,
			);
			if (!check || applyStateRef.current.readOnly) return;
			if (source === baselineRef.current) {
				setCheckState({ forText: source, response: IN_SYNC_CHECK_RESPONSE });
				return;
			}
			setChecking(true);
			try {
				const response = await check(
					appIdRef.current,
					boardIdRef.current,
					source,
					scopeAnchorsRef.current,
					applyModuleIdRef.current,
				);
				if (editor.getModel() === model && model.getValue() === source) {
					setCheckState({ forText: source, response });
				}
			} catch {
				if (editor.getModel() === model && model.getValue() === source) {
					setCheckState(undefined);
				}
			} finally {
				setChecking(false);
			}
		}, LINT_DEBOUNCE_MS);
		return () => clearTimeout(handle);
	}, [
		text,
		baseline,
		catalogNodes,
		backend,
		editorReady,
		namesReady,
		boardScopeKey,
	]);

	const toggleFullScreen = useCallback(() => {
		setFullScreen((value) => !value);
		requestAnimationFrame(() => editorRef.current?.layout());
	}, []);

	useEffect(() => {
		if (!fullScreen) return;
		const container = containerRef.current;
		if (!container) return;
		const onKeyDown = (event: KeyboardEvent) => {
			// Monaco marks the Escapes it consumes (find widget, suggest) as handled.
			if (event.key !== "Escape" || event.defaultPrevented) return;
			setFullScreen(false);
			requestAnimationFrame(() => editorRef.current?.layout());
		};
		container.addEventListener("keydown", onKeyDown);
		return () => container.removeEventListener("keydown", onKeyDown);
	}, [fullScreen]);

	const handleCopy = useCallback(async () => {
		await navigator.clipboard.writeText(textRef.current);
		toast.success(
			t("flowscriptCopiedToClipboard", "FlowScript copied to clipboard"),
		);
	}, [t]);

	const requestReload = useCallback(() => {
		if (dirtyRef.current) {
			setRefreshConfirmationOpen(true);
			return;
		}
		void load();
	}, [load]);

	const requestExitScope = useCallback(() => {
		if (!onExitScopeRef.current) return;
		if (dirtyRef.current) {
			exitScopeAfterConfirmRef.current = true;
			setRefreshConfirmationOpen(true);
			return;
		}
		onExitScopeRef.current();
	}, []);

	const toggleDimAnchors = useCallback(() => {
		setDimAnchors((value) => {
			const next = !value;
			writeDimAnchorsPreference(next);
			return next;
		});
	}, []);

	const runFormat = useCallback(() => {
		void editorRef.current?.getAction("editor.action.formatDocument")?.run();
	}, []);

	const editorTheme = useMemo(
		() =>
			resolvedTheme === "dark" ? FLOWSCRIPT_THEME_DARK : FLOWSCRIPT_THEME_LIGHT,
		[resolvedTheme],
	);

	const formatSupported = Boolean(backend.boardState.formatFlowScript);

	const previewFresh =
		typeof checkState !== "undefined" && checkState.forText === text;
	const previewResponse = previewFresh ? checkState.response : undefined;
	const previewCommands = previewResponse?.board_commands;
	const previewCounts = useMemo(() => {
		if (!previewResponse) return undefined;
		if (previewCommands) return summarizeBoardCommands(previewCommands);
		// Older backends report only a count — surface it as generic updates.
		return {
			addedNodes: 0,
			removedNodes: 0,
			wires: 0,
			removedWires: 0,
			variables: 0,
			layers: 0,
			comments: 0,
			updates: previewResponse.command_count,
			total: previewResponse.command_count,
		};
	}, [previewResponse, previewCommands]);
	const previewErrorCount =
		previewResponse &&
		(!previewResponse.parse_valid || !previewResponse.reconcile_valid)
			? Math.max(previewResponse.diagnostics.length, 1)
			: 0;
	const previewDestructiveCount = useMemo(
		() =>
			previewCommands ? destructiveCommandSummaries(previewCommands).length : 0,
		[previewCommands],
	);
	// Last-writer-wins warning BEFORE apply: locally edited statements that a
	// remote change also touched since the local baseline. Only computed while
	// a merge actually recorded remote-touched anchors.
	const remoteOverlapCount = useMemo(() => {
		if (remoteTouched.size === 0 || !dirty) return 0;
		return intersectRemoteTouched(
			remoteTouched,
			deriveClaimedAnchorIds(baseline, text),
		).length;
	}, [remoteTouched, baseline, text, dirty]);

	// File tabs only where a file can actually be rendered and applied on its own: a backend
	// without `getFlowScriptFile` keeps the whole-board document, where they would mean nothing.
	// The open file's dirty flag lives here; the other files' comes from the board-owned stash.
	const fileTabsVisible =
		hasModules && Boolean(backend.boardState.getFlowScriptFile);
	const fileTabs = useMemo(
		() =>
			fileTabsVisible && modules
				? flowScriptFileTabs(
						modules,
						fileId,
						dirty && !readOnly,
						files?.dirtyFileIds ?? EMPTY_DIRTY_FILES,
					)
				: [],
		[fileTabsVisible, modules, fileId, dirty, readOnly, files?.dirtyFileIds],
	);

	const scopedSectionCount = scopeAnchors?.length ?? 0;
	// Peers whose broadcast scope equals ours (set equality on node ids) — shown
	// as "with NAME" in the scoped banner. Sessions stay independent: a peer
	// leaving this list never affects the local scope.
	const scopeSharers = useMemo(
		() =>
			scoped && scopeMode.kind === "scoped"
				? peersSharingFlowScriptScope(
						presenceSnapshot.scopes,
						scopeMode.nodeIds,
					)
				: [],
		[scoped, scopeMode, presenceSnapshot.scopes],
	);
	const scopeSharerName =
		scopeSharers.length > 0
			? scopeSharers[0].self
				? t("you", "You")
				: ((scopeSharers[0].sub
						? peerUsers?.get(scopeSharers[0].sub)?.truncatedName
						: undefined) ?? t("common:user", "User"))
			: undefined;

	return (
		<div
			ref={containerRef}
			className={
				fullScreen
					? "fixed inset-0 z-50 flex min-h-0 flex-col bg-background"
					: "flex h-full min-h-0 w-full flex-col bg-background"
			}
		>
			<div className="flex items-center justify-between gap-2 border-b px-3 py-2">
				<div className="flex min-w-0 items-center gap-2">
					<FileCode2Icon className="h-4 w-4 shrink-0 text-primary" />
					<span className="truncate text-sm font-medium">
						{t("flowscript", "FlowScript")}
					</span>
					{readOnly && (
						<Badge variant="secondary" className="text-[10px]">
							v{version?.join(".")} {t("readonly2", "— read-only")}
						</Badge>
					)}
					{dirty && !readOnly && (
						<span
							className="h-2 w-2 shrink-0 rounded-full bg-primary"
							title={t("unappliedChanges", "Unapplied changes")}
						/>
					)}
					{!readOnly && (
						<FlowScriptApplyPreviewChip
							state={{
								counts: previewCounts,
								errorCount: previewErrorCount,
								destructiveCount: previewDestructiveCount,
								checking,
								remoteOverlapCount,
							}}
							expanded={previewExpanded}
							onToggle={() => setPreviewExpanded((value) => !value)}
						/>
					)}
				</div>
				<div className="flex items-center gap-1">
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className={`h-7 w-7 ${dimAnchors ? "" : "text-primary"}`}
								onClick={toggleDimAnchors}
							>
								<AnchorIcon className="h-3.5 w-3.5" />
							</Button>
						</TooltipTrigger>
						<TooltipContent>
							{dimAnchors
								? t("showAnchorsAtFullOpacity", "Show anchors at full opacity")
								: t("dimAnchorComments", "Dim anchor comments")}
						</TooltipContent>
					</Tooltip>
					{developerMode && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className={`h-7 w-7 ${presenceDebug ? "text-primary" : ""}`}
									onClick={togglePresenceDebug}
								>
									<BugIcon className="h-3.5 w-3.5" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{t("flowscriptPresenceDebug", "Presence diagnostics")}
							</TooltipContent>
						</Tooltip>
					)}
					{formatSupported && !readOnly && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-7 w-7"
									disabled={loading || applying}
									onClick={runFormat}
								>
									<WandSparklesIcon className="h-3.5 w-3.5" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{t("formatFlowscript", "Format (⇧⌥F)")}
							</TooltipContent>
						</Tooltip>
					)}
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-7 w-7"
								onClick={handleCopy}
							>
								<CopyIcon className="h-3.5 w-3.5" />
							</Button>
						</TooltipTrigger>
						<TooltipContent>{t("copySource", "Copy source")}</TooltipContent>
					</Tooltip>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-7 w-7"
								disabled={loading || applying}
								onClick={requestReload}
							>
								<RefreshCcwIcon className="h-3.5 w-3.5" />
							</Button>
						</TooltipTrigger>
						<TooltipContent>
							{t("reRenderFromBoard", "Re-render from board")}
						</TooltipContent>
					</Tooltip>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className={`h-7 w-7 ${fullScreen ? "text-primary" : ""}`}
								onClick={toggleFullScreen}
							>
								{fullScreen ? (
									<Minimize2Icon className="h-3.5 w-3.5" />
								) : (
									<Maximize2Icon className="h-3.5 w-3.5" />
								)}
							</Button>
						</TooltipTrigger>
						<TooltipContent>
							{fullScreen
								? t("flowscriptExitFullScreen", "Exit full screen (Esc)")
								: t("flowscriptFullScreen", "Full screen")}
						</TooltipContent>
					</Tooltip>
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7"
						onClick={onClose}
					>
						<XIcon className="h-3.5 w-3.5" />
					</Button>
				</div>
			</div>

			{onSelectFile && (
				<FlowScriptFileTabs
					tabs={fileTabs}
					activeFileId={fileId}
					// A selection scope is not a file: leave it before switching documents.
					disabled={applying || scoped}
					onSelect={onSelectFile}
				/>
			)}

			{scoped && !loadError && (
				<output className="flex flex-wrap items-center justify-between gap-2 border-b bg-[color-mix(in_oklch,var(--primary)_8%,transparent)] px-3 py-2 text-xs text-muted-foreground">
					<span className="flex min-w-0 items-center gap-2">
						<FocusIcon className="h-3.5 w-3.5 shrink-0 text-primary" />
						{typeof totalSections === "number" && totalSections > 0
							? t("flowscriptScopedBanner", {
									defaultValue:
										"Editing {{selected}} of {{total}} sections — out-of-scope content is untouched",
									selected: scopedSectionCount,
									total: totalSections,
								})
							: t("flowscriptScopedBannerNoTotal", {
									defaultValue:
										"Editing {{selected}} selected sections — out-of-scope content is untouched",
									selected: scopedSectionCount,
								})}
						{scopeSharerName && (
							<Tooltip>
								<TooltipTrigger asChild>
									<span className="shrink-0 font-medium text-primary">
										{scopeSharers.length > 1
											? t("flowscriptScopedWithOthers", {
													defaultValue: "with {{name}} +{{count}}",
													name: scopeSharerName,
													count: scopeSharers.length - 1,
												})
											: t("flowscriptScopedWith", {
													defaultValue: "with {{name}}",
													name: scopeSharerName,
												})}
									</span>
								</TooltipTrigger>
								<TooltipContent side="bottom" className="max-w-64 text-xs">
									{t(
										"flowscriptScopedWithTooltip",
										"Sessions in a shared scope are independent — a teammate exiting or closing theirs never ejects you.",
									)}
								</TooltipContent>
							</Tooltip>
						)}
					</span>
					<Button
						variant="outline"
						size="sm"
						className="h-7 shrink-0 px-2 text-xs"
						disabled={loading || applying}
						onClick={requestExitScope}
					>
						{t("editWholeBoard", "Edit whole board")}
					</Button>
				</output>
			)}

			{boardChangedBehindEdits && (
				<div
					role="alert"
					className="flex flex-wrap items-center justify-between gap-2 border-b bg-[color-mix(in_oklch,var(--primary)_8%,transparent)] px-3 py-2 text-xs text-muted-foreground"
				>
					<span className="flex min-w-0 items-center gap-2">
						<AlertTriangleIcon className="h-3.5 w-3.5 shrink-0 text-yellow-500" />
						{t(
							"boardChangedApplyingPausedUntilRefresh",
							"The board changed while you were editing. Applying is paused until you refresh from the board.",
						)}
					</span>
					<div className="flex shrink-0 items-center gap-1.5">
						<Button
							variant="ghost"
							size="sm"
							className="h-7 px-2 text-xs"
							onClick={() => void handleCopy()}
						>
							<CopyIcon className="mr-1 h-3 w-3" />
							{t("copyEdits", "Copy edits")}
						</Button>
						<Button
							variant="outline"
							size="sm"
							className="h-7 px-2 text-xs"
							disabled={loading || applying}
							onClick={requestReload}
						>
							<RefreshCcwIcon className="mr-1 h-3 w-3" />
							{t("refreshFromBoard", "Refresh from board")}
						</Button>
					</div>
				</div>
			)}

			{mergeConflicts.length > 0 && (
				<div
					role="alert"
					className="flex flex-wrap items-center justify-between gap-2 border-b bg-[color-mix(in_oklch,var(--primary)_8%,transparent)] px-3 py-2 text-xs text-muted-foreground"
				>
					<span className="flex min-w-0 items-center gap-2">
						<AlertTriangleIcon className="h-3.5 w-3.5 shrink-0 text-yellow-500" />
						{t("flowscriptMergeConflicts", {
							defaultValue_one:
								"{{count}} statement changed on the board and in your draft — resolve it inline or choose a side",
							defaultValue_other:
								"{{count}} statements changed on the board and in your draft — resolve them inline or choose a side",
							count: mergeConflicts.length,
						})}
					</span>
					<div className="flex shrink-0 items-center gap-1.5">
						<Button
							variant="ghost"
							size="sm"
							className="h-7 px-2 text-xs"
							onClick={() => void copyPreMergeVersion()}
						>
							<CopyIcon className="mr-1 h-3 w-3" />
							{t("flowscriptCopyMyVersion", "Copy my version")}
						</Button>
						<Button
							variant="outline"
							size="sm"
							className="h-7 px-2 text-xs"
							onClick={() => resolveAllConflicts("mine")}
						>
							{t("flowscriptKeepAllMine", "Keep all mine")}
						</Button>
						<Button
							variant="outline"
							size="sm"
							className="h-7 px-2 text-xs"
							onClick={() => resolveAllConflicts("theirs")}
						>
							{t("flowscriptTakeAllTheirs", "Take all theirs")}
						</Button>
					</div>
				</div>
			)}

			{previewExpanded && previewResponse && (
				<FlowScriptApplyPreviewList
					commands={previewCommands ?? []}
					diagnostics={previewErrorCount > 0 ? previewResponse.diagnostics : []}
					remoteTouchedIds={remoteTouched}
				/>
			)}

			<div className="relative min-h-0 flex-1">
				{loading && (
					<div className="absolute inset-0 z-10 flex items-center justify-center bg-background/60">
						<Loader2Icon className="h-5 w-5 animate-spin text-muted-foreground" />
					</div>
				)}
				{loadError ? (
					<div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
						<AlertTriangleIcon className="h-6 w-6 text-destructive" />
						<p className="text-sm text-muted-foreground">{loadError}</p>
						<Button variant="outline" size="sm" onClick={() => void load()}>
							{t("retry", "Retry")}
						</Button>
					</div>
				) : (
					<Editor
						height="100%"
						className={FLOW_KEY_OPT_OUT_CLASS}
						language={FLOWSCRIPT_LANGUAGE_ID}
						value={text}
						onChange={(value) => setText(value ?? "")}
						theme={editorTheme}
						onMount={handleEditorMount}
						options={{
							readOnly,
							minimap: { enabled: true },
							fontSize: 12,
							fontFamily:
								"'SF Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace",
							fontLigatures: true,
							scrollBeyondLastLine: false,
							automaticLayout: true,
							wordWrap: "off",
							tabSize: 4,
							padding: { top: 8, bottom: 8 },
							folding: true,
							renderLineHighlight: "line",
							smoothScrolling: true,
							quickSuggestions: true,
							suggestOnTriggerCharacters: true,
							tabCompletion: "on",
							suggestSelection: "recentlyUsedByPrefix",
							parameterHints: { enabled: true },
						}}
					/>
				)}
				{developerMode && presenceDebug && !loadError && (
					<FlowScriptPresenceDebug
						awareness={awareness}
						snapshot={presenceSnapshot}
						anchorIndex={anchorIndex}
						enabled={!readOnly}
						hasTextFocus={editorHasTextFocus}
					/>
				)}
				{commentThreadState && commentsEnabled && !loading && !loadError && (
					<FlowScriptCommentOverlay
						editor={editorReady ? editorRef.current : null}
						monaco={monacoRef.current}
						anchorId={commentThreadState.anchorId}
						line={
							commentModel.threadsByAnchorId.get(commentThreadState.anchorId)
								?.line ??
							anchorIndex.firstLineById.get(commentThreadState.anchorId) ??
							commentThreadState.line
						}
						comments={
							commentModel.threadsByAnchorId.get(commentThreadState.anchorId)
								?.comments ?? []
						}
						authors={commentAuthors}
						sub={sub}
						editable={commentsEditable}
						focusComposer={commentThreadState.focusComposer}
						onCreate={handleCreateComment}
						onUpdate={handleUpdateComment}
						onDelete={handleDeleteComment}
						onClose={closeCommentThread}
					/>
				)}
			</div>

			{diagnostics.length > 0 && (
				<div className="max-h-28 shrink-0 overflow-y-auto border-t px-3 py-2">
					<div className="mb-1 flex items-center justify-between">
						<span className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
							<AlertTriangleIcon className="h-3 w-3 text-yellow-500" />
							{t("countWarnings", {
								defaultValue_one: "{{count}} warning",
								defaultValue_other: "{{count}} warnings",
								count: diagnostics.length,
							})}
						</span>
						<Button
							variant="ghost"
							size="icon"
							className="h-5 w-5"
							onClick={() => setDiagnostics([])}
						>
							<XIcon className="h-3 w-3" />
						</Button>
					</div>
					<ul className="space-y-1">
						{diagnostics.map((diagnostic) => (
							<li
								key={diagnostic}
								className="text-xs text-muted-foreground break-words"
							>
								{diagnostic}
							</li>
						))}
					</ul>
				</div>
			)}

			{!readOnly && (
				<div className="flex shrink-0 items-center justify-between gap-2 border-t px-3 py-2">
					<span className="text-[11px] text-muted-foreground">
						{boardChangedBehindEdits
							? t(
									"boardChangedRefreshBeforeApplying",
									"Board changed — refresh before applying",
								)
							: mergeConflicts.length > 0
								? t(
										"flowscriptResolveConflictsBeforeApply",
										"Resolve the merge conflicts before applying",
									)
								: dirty
									? t(
											"unappliedChangesSToApply",
											"Unapplied changes — ⌘S to apply",
										)
									: t("inSyncWithBoard", "In sync with board")}
					</span>
					<div className="flex items-center gap-2">
						<Button
							variant="ghost"
							size="sm"
							className="h-7"
							disabled={!dirty || applying}
							onClick={() => {
								setText(baseline);
								setDiagnostics([]);
								// The baseline IS the board's latest render — a reset settles
								// every outstanding merge conflict as "theirs".
								setMergeConflicts([]);
							}}
						>
							<Undo2Icon className="mr-1 h-3.5 w-3.5" />
							{t("reset", "Reset")}
						</Button>
						<Button
							size="sm"
							className="h-7"
							disabled={!canApply}
							onClick={requestApply}
						>
							{applying ? (
								<Loader2Icon className="mr-1 h-3.5 w-3.5 animate-spin" />
							) : null}
							{t("applyToBoard", "Apply to board")}
						</Button>
					</div>
				</div>
			)}

			<AlertDialog
				open={refreshConfirmationOpen}
				onOpenChange={(open) => {
					setRefreshConfirmationOpen(open);
					if (!open) exitScopeAfterConfirmRef.current = false;
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t(
								"refreshFlowscriptFromTheBoard",
								"Refresh FlowScript from the board?",
							)}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"thisReplacesTheCurrentEditorTextWithTheLatestBoardStateCopyYourEditsFirstIfYouWantToReapplyThemToTheRefreshedScript",
								"This replaces the current editor text with the latest board state. Copy your edits first if you want to reapply them to the refreshed script.",
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("keepEditing", "Keep editing")}
						</AlertDialogCancel>
						<Button variant="outline" onClick={() => void handleCopy()}>
							<CopyIcon className="mr-1 h-3.5 w-3.5" />
							{t("copyEdits", "Copy edits")}
						</Button>
						<AlertDialogAction
							onClick={() => {
								setRefreshConfirmationOpen(false);
								if (exitScopeAfterConfirmRef.current) {
									exitScopeAfterConfirmRef.current = false;
									onExitScopeRef.current?.();
									return;
								}
								void load();
							}}
						>
							{t("refreshAndReplace", "Refresh and replace")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>

			<AlertDialog
				open={typeof pendingDeletions !== "undefined"}
				onOpenChange={(open) => {
					if (!open) setPendingDeletions(undefined);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t(
								"thisEditDeletesExistingBoardItems",
								"This edit deletes existing board items",
							)}
						</AlertDialogTitle>
						<AlertDialogDescription asChild>
							<div>
								<p>
									{t("flowscriptApplyDeletes", {
										defaultValue_one:
											"Applying this FlowScript deletes {{count}} existing board item:",
										defaultValue_other:
											"Applying this FlowScript deletes {{count}} existing board items:",
										count: pendingDeletions?.length ?? 0,
									})}
								</p>
								<ul className="mt-2 max-h-40 list-disc space-y-1 overflow-y-auto pl-5">
									{pendingDeletions?.map((deletion) => (
										<li
											key={`${deletion.kind}-${deletion.label}`}
											className="break-words"
										>
											{deletion.label}
										</li>
									))}
								</ul>
							</div>
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("keepEverything", "Keep everything")}
						</AlertDialogCancel>
						<AlertDialogAction
							onClick={() => {
								setPendingDeletions(undefined);
								void runApply(true);
							}}
						>
							{t("applyWithDeletions", "Apply with deletions")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>

			<AlertDialog
				open={typeof destructiveMessage !== "undefined"}
				onOpenChange={(open) => {
					if (!open) setDestructiveMessage(undefined);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t(
								"thisEditDeletesExistingBoardItems",
								"This edit deletes existing board items",
							)}
						</AlertDialogTitle>
						<AlertDialogDescription className="break-words">
							{destructiveMessage}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("keepEverything", "Keep everything")}
						</AlertDialogCancel>
						<AlertDialogAction
							onClick={() => {
								setDestructiveMessage(undefined);
								void runApply(true);
							}}
						>
							{t("applyWithDeletions", "Apply with deletions")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}
