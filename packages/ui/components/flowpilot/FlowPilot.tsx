"use client";

import { createId } from "@paralleldrive/cuid2";
import { AnimatePresence, motion } from "framer-motion";
import {
	ArrowDown,
	CameraIcon,
	CheckCircle2,
	ChevronDownIcon,
	CircleDashedIcon,
	ClockIcon,
	FileCode2Icon,
	FileDiffIcon,
	ImageIcon,
	LayoutGridIcon,
	ListTreeIcon,
	Loader2,
	SendIcon,
	SparklesIcon,
	SquarePenIcon,
	WorkflowIcon,
	WrenchIcon,
	XIcon,
	ZapIcon,
} from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useCopilotSDK, useInvoke } from "../../hooks";
import { copilotBackendConnectionCoordinator } from "../../hooks/copilot-backend-coordinator";
import { useFrontendRuntimeToolExecutor } from "../../hooks/use-frontend-runtime-tool-executor";
import { IBitTypes, filterHostableLlmModels, isFreeLlmModel } from "../../lib";
import { shouldSkipUnavailableCreateTableApproval } from "../../lib/database-capability-session";
import { flowPilotCommandApplyDiagnostics } from "../../lib/flowpilot-command-apply";
import { buildFlowPilotBoardContextAugmentation } from "../../lib/flowpilot/board-context-manifest";
import {
	classifyAgentBackendError,
	formatAgentBackendDiagnostic,
	shouldPersistAgentBackendDiagnostic,
} from "../../lib/flowpilot/agent-backend-diagnostics";
import {
	DIRECT_FLOWPILOT_BOARD_EDIT_REQUEST_PREFIX,
	boardEditJobResolutionHistoryMode,
	deliverBoardEditJobReceipt,
	isDirectFlowPilotBoardEditJob,
} from "../../lib/flowpilot/board-edit-job-delivery";
import {
	type IFlowPilotConversation,
	addMessage,
	createConversation,
	getMessages,
	updateConversation,
	updateMessage,
} from "../../lib/flowpilot-db";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";

import { Button } from "../ui/button";
import { Checkbox } from "../ui/checkbox";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../ui/collapsible";
import {
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { ScrollArea } from "../ui/scroll-area";
import { Textarea } from "../ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

import { ContextNodes } from "./ContextNodes";
import { HistoryPanel } from "./HistoryPanel";
import { MessageContent } from "./MessageContent";
import { PendingCommandsView } from "./PendingCommandsView";
import { PendingComponentsView } from "./PendingComponentsView";
import { StatusPill } from "./StatusPill";
import { resolveFrontendToolExecutionDeadline } from "./board-edit-guard";
import { releaseReturnedFlowIrCommitBeforeStaleResponse } from "./copilot-request-context";
import {
	type CopilotStreamEvent,
	appendBoundedStreamDetail,
	createCopilotStreamParser,
} from "./copilot-stream-parser";
import {
	type FlowScriptWorkspaceCandidate,
	extractFlowScriptWorkspaceCandidates,
	isFlowScriptWorkspaceApplicable,
	rememberFlowScriptWorkspaceCandidate,
	resolveFinalFlowScriptWorkspaceCandidate,
} from "./flowscript-workspace-candidates";
import {
	FlowScriptWorkspacePanel,
	formatLineCount,
} from "./flowscript-workspace-panel";
import {
	FrontendToolRequestGuard,
	type FrontendToolRequestLease,
} from "./frontend-tool-request-guard";
import { FlowPilotGenerationMetricsRun } from "./generation-metrics";
import { flowPilotPanelConversationId } from "./panel-conversation-scope";
import { buildBudgetedHistory } from "./history-budget";
import {
	InlineFlowScriptPreview,
	type InlineFlowScriptPreviewValue,
	flowScriptWorkspaceOwnsApply,
	isDraftingFlowScriptWorkspace,
	resolveDisplayedFlowScriptPreview,
	resolveLiveFlowScriptPreviewForMessage,
} from "./inline-flowscript-preview";
import {
	type ProviderModelPickerProvider,
	ProviderModelReasoningPicker,
} from "./provider-model-reasoning-picker";
import type {
	AIProvider,
	AgentBackendProvider,
	AgentMode,
	AttachedImage,
	CopilotMessage,
	FlowPilotProcessEvent,
	FlowPilotProcessEventStatus,
	FlowPilotProps,
	LoadingPhase,
	UnifiedPlanStep,
} from "./types";
import {
	flowPilotModelIdForProvider,
	isAgentBackendProvider,
	normalizeAIProvider,
} from "./types";
import { getCommandSummary } from "./utils";
import {
	validateCanvasSettings,
	validateComponents,
} from "./validateComponents";

import type {
	BoardEditJob,
	CanvasSettings,
	CopilotScope,
	FlowIrCommitDisposition,
	FlowIrCommitDispositionResult,
	FlowIrCommitToken,
	UnifiedChatMessage,
} from "../../lib/schema/copilot";
import type { BoardCommand, Suggestion } from "../../lib/schema/flow/copilot";
import type { SurfaceComponent } from "../a2ui/types";

const MAX_ATTACHED_IMAGES = 4;
const MAX_ATTACHED_IMAGE_BYTES = 5 * 1024 * 1024;
const ALLOWED_ATTACHED_IMAGE_TYPES = new Set([
	"image/png",
	"image/jpeg",
	"image/webp",
	"image/gif",
]);

const DESTRUCTIVE_FLOWSCRIPT_DIAGNOSTIC_PREFIX =
	"FlowScript edit would delete ";
const FLOW_IR_DISMISS_RETRY_DELAYS_MS = [0, 250, 1_000, 3_000] as const;
const FLOWSCRIPT_DRAFT_PREVIEW_INTERVAL_MS = 80;
const BOARD_EDIT_JOB_POLL_INTERVAL_MS = 2_500;
const HOST_BOARD_APPLY_FEEDBACK_PREFIX = "Host board apply result:";
const HOST_BOARD_APPLY_FAILURE_PREFIX = `${HOST_BOARD_APPLY_FEEDBACK_PREFIX}\nThe queued board change was not fully applied.`;

function isSettledBoardEditJob(job: BoardEditJob): boolean {
	return ["applied", "denied", "stale", "cancelled"].includes(job.phase);
}

function isHostBoardApplyFeedbackMessage(message: CopilotMessage): boolean {
	return (
		message.role === "assistant" &&
		message.content.startsWith(HOST_BOARD_APPLY_FEEDBACK_PREFIX)
	);
}

function latestUnresolvedBoardApplyFeedback(
	messages: CopilotMessage[],
): string {
	for (let index = messages.length - 1; index >= 0; index--) {
		const message = messages[index];
		if (!isHostBoardApplyFeedbackMessage(message)) continue;
		return message.content.startsWith(HOST_BOARD_APPLY_FAILURE_PREFIX)
			? message.content
			: "";
	}
	return "";
}

function applyResultDiagnostics(applyResult: unknown): string[] {
	if (!applyResult || typeof applyResult !== "object") return [];
	const diagnostics = (applyResult as { diagnostics?: unknown }).diagnostics;
	return Array.isArray(diagnostics)
		? diagnostics.filter((diagnostic): diagnostic is string => {
				return typeof diagnostic === "string";
			})
		: [];
}

function applyResultCommandCount(applyResult: unknown): number | undefined {
	if (!applyResult || typeof applyResult !== "object") return undefined;
	const commands = (applyResult as { commands?: unknown }).commands;
	return Array.isArray(commands) ? commands.length : undefined;
}

function applyResultBoardCommands(applyResult: unknown): BoardCommand[] {
	if (!applyResult || typeof applyResult !== "object") return [];
	const boardCommands = (applyResult as { board_commands?: unknown })
		.board_commands;
	return Array.isArray(boardCommands) ? (boardCommands as BoardCommand[]) : [];
}

function flowPilotBoardNodeCount(
	board: FlowPilotProps["board"],
): number | undefined {
	if (!board) return undefined;
	const nodeIds = new Set(Object.keys(board.nodes ?? {}));
	for (const layer of Object.values(board.layers ?? {})) {
		for (const nodeId of Object.keys(layer?.nodes ?? {})) nodeIds.add(nodeId);
	}
	return nodeIds.size;
}

function destructiveFlowScriptDiagnostic(diagnostics: string[]): string | null {
	return (
		diagnostics.find((diagnostic) =>
			diagnostic.startsWith(DESTRUCTIVE_FLOWSCRIPT_DIAGNOSTIC_PREFIX),
		) ?? null
	);
}

function canAttachImage(file: File): boolean {
	return (
		ALLOWED_ATTACHED_IMAGE_TYPES.has(file.type) &&
		file.size <= MAX_ATTACHED_IMAGE_BYTES
	);
}

function normalizedDataUrlImageType(mediaType: string): string | null {
	if (mediaType === "image/jpg") return "image/jpeg";
	return ALLOWED_ATTACHED_IMAGE_TYPES.has(mediaType) ? mediaType : null;
}

function base64ByteLength(data: string): number {
	const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
	return Math.max(0, Math.floor((data.length * 3) / 4) - padding);
}

function parseStreamJson(payload: string): Record<string, unknown> | null {
	try {
		const parsed = JSON.parse(payload);
		return parsed && typeof parsed === "object" ? parsed : null;
	} catch {
		return null;
	}
}

function getProcessToolLabel(toolName?: string): string {
	if (!toolName) return "Using tool";
	switch (toolName) {
		case "think":
		case "analyze":
			return "Thinking";
		case "get_current_flowscript":
			return "Reading current FlowScript";
		case "get_declarations":
			return "Looking up FlowScript declarations";
		case "write_flowscript":
			return "Writing FlowScript";
		case "patch_flowscript":
			return "Repairing FlowScript";
		case "check_flowscript":
			return "Checking FlowScript";
		case "commit_flowscript":
			return "Queueing checked FlowScript";
		case "edit_flowscript":
			return "Editing FlowScript";
		case "catalog_search":
			return "Searching catalog";
		case "emit_commands":
			return "Queueing board changes";
		case "emit_ui":
		case "emit_surface":
			return "Generating UI";
		case "get_node_details":
			return "Reading node details";
		case "list_board_nodes":
			return "Inspecting board";
		case "get_unconfigured_nodes":
			return "Checking missing inputs";
		case "internet_search":
			return "Searching web";
		case "open_url":
			return "Reading web source";
		case "archive_lookup":
			return "Checking web archive";
		case "database_tool":
			return "Using database";
		case "storage_tool":
			return "Using storage";
		case "ui_inspect":
			return "Inspecting pages & widgets";
		case "execute_event":
			return "Executing event";
		case "execute_node":
			return "Executing node";
		case "query_execution_logs":
			return "Reading execution logs";
		case "ask_user":
			return "Asking for input";
		default:
			return toolName.replace(/_/g, " ");
	}
}

function processStatusFromPlanStepStatus(
	status?: UnifiedPlanStep["status"],
): FlowPilotProcessEventStatus {
	switch (status) {
		case "InProgress":
			return "running";
		case "Completed":
			return "done";
		case "Failed":
			return "error";
		default:
			return "info";
	}
}

function formatProcessElapsed(
	event: FlowPilotProcessEvent,
): string | undefined {
	const end =
		event.updatedAt ??
		(event.status === "running" ? Date.now() : event.createdAt);
	const elapsed = end - event.createdAt;
	if (!Number.isFinite(elapsed) || elapsed < 1_000) return undefined;
	const seconds = Math.round(elapsed / 1_000);
	if (seconds < 60) return `${seconds}s`;
	return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

function stringifyPreview(value: unknown): string | undefined {
	if (typeof value === "string") return value;
	if (value === undefined || value === null) return undefined;
	try {
		return JSON.stringify(value, null, 2);
	} catch {
		return String(value);
	}
}

type FrontendToolApprovalKind = "none" | "mutating" | "execute";

interface FrontendToolApproval {
	kind: FrontendToolApprovalKind;
	title?: string;
	description?: string;
	sessionKey?: string;
}

interface FrontendToolRequest {
	requestId: string;
	toolName: string;
	arguments: Record<string, unknown>;
	approval?: FrontendToolApproval;
	deadlineAtMs?: number;
	deadline_at_ms?: number;
}

interface FrontendToolResponse {
	requestId: string;
	approved: boolean;
	result?: unknown;
	error?: string;
}

interface FrontendToolChoice {
	label: string;
	value?: unknown;
	description?: string;
}

type FrontendToolDialogState =
	| {
			type: "approval";
			request: FrontendToolRequest;
			remember: boolean;
	  }
	| {
			type: "ask";
			request: FrontendToolRequest;
			mode: "freeform" | "single_choice" | "multiple_choice";
			answer: string;
			selected: Set<number>;
	  };

interface FrontendToolQueuedDialog {
	dialog: FrontendToolDialogState;
	resolve: (value: any) => void;
}

type TauriCoreModule = {
	invoke: <T = unknown>(
		command: string,
		args?: Record<string, unknown>,
	) => Promise<T>;
};

type TauriEventModule = {
	listen: <T = unknown>(
		event: string,
		handler: (event: { payload: T }) => void | Promise<void>,
	) => Promise<() => void>;
};

const FLOWPILOT_FRONTEND_TOOL_EVENT = "flowpilot://frontend-tool-request";
const FLOWPILOT_FRONTEND_TOOL_CANCEL_EVENT = "flowpilot://frontend-tool-cancel";

function isTauriRuntime(): boolean {
	if (typeof window === "undefined") return false;
	const w = window as unknown as Record<string, unknown>;
	return Boolean(w.__TAURI__ || w.__TAURI_IPC__ || w.__TAURI_INTERNALS__);
}

async function importTauriCore(): Promise<TauriCoreModule> {
	return import("@tauri-apps/api/core") as Promise<TauriCoreModule>;
}

async function importTauriEvent(): Promise<TauriEventModule> {
	return import("@tauri-apps/api/event") as Promise<TauriEventModule>;
}

function getArgString(
	args: Record<string, unknown>,
	snake: string,
	camel = snake,
): string | undefined {
	const value = args[snake] ?? args[camel];
	return typeof value === "string" && value.trim() ? value : undefined;
}

type FlowScriptDiffLine = {
	type: "added" | "removed" | "context";
	text: string;
};

function buildFlowScriptDiff(
	before: string | undefined,
	after: string,
): FlowScriptDiffLine[] {
	const beforeLines = before ? before.split("\n") : [];
	const afterLines = after.split("\n");

	if (!before || beforeLines.length === 0) {
		return afterLines.map((text) => ({ type: "added", text }));
	}

	if (before === after) {
		return afterLines.slice(0, 80).map((text) => ({ type: "context", text }));
	}

	let start = 0;
	while (
		start < beforeLines.length &&
		start < afterLines.length &&
		beforeLines[start] === afterLines[start]
	) {
		start += 1;
	}

	let beforeEnd = beforeLines.length - 1;
	let afterEnd = afterLines.length - 1;
	while (
		beforeEnd >= start &&
		afterEnd >= start &&
		beforeLines[beforeEnd] === afterLines[afterEnd]
	) {
		beforeEnd -= 1;
		afterEnd -= 1;
	}

	const contextBefore = beforeLines
		.slice(Math.max(0, start - 2), start)
		.map((text) => ({ type: "context" as const, text }));
	const removed = beforeLines
		.slice(start, beforeEnd + 1)
		.map((text) => ({ type: "removed" as const, text }));
	const added = afterLines
		.slice(start, afterEnd + 1)
		.map((text) => ({ type: "added" as const, text }));
	const contextAfter = afterLines
		.slice(afterEnd + 1, Math.min(afterLines.length, afterEnd + 3))
		.map((text) => ({ type: "context" as const, text }));

	return [...contextBefore, ...removed, ...added, ...contextAfter].slice(
		0,
		120,
	);
}

function FlowPilotImpl({
	agentMode,
	title = "FlowPilot",
	className,
	onClose,
	onWorkspaceVisibleChange,
	// Provider props
	forceProvider,
	defaultProvider = "bits",
	// Board mode props
	appId,
	board,
	catalogNodes,
	selectedNodeIds = [],
	onAcceptSuggestion,
	onExecuteCommands,
	onApplyFlowScript,
	onApplyFlowIrCommit,
	onFocusNode,
	onSelectNodes,
	runContext,
	initialPrompt,
	// UI mode props
	currentComponents = [],
	selectedComponentIds = [],
	onComponentsGenerated,
	onApplyComponents,
	// Screenshot prop
	captureScreenshot,
}: FlowPilotProps) {
	// Core state
	const [messages, setMessages] = useState<CopilotMessage[]>([]);
	const [input, setInput] = useState("");
	const [loading, setLoading] = useState(false);
	const [loadingPhase, setLoadingPhase] = useState<LoadingPhase>("idle");
	const [loadingStartTime, setLoadingStartTime] = useState<number | null>(null);
	const [planSteps, setPlanSteps] = useState<UnifiedPlanStep[]>([]);
	const [attachedImages, setAttachedImages] = useState<AttachedImage[]>([]);
	const [userScrolledUp, setUserScrolledUp] = useState(false);
	const [selectedModelId, setSelectedModelId] = useState("");
	const [selectedReasoningEffort, setSelectedReasoningEffort] = useState("");

	// Provider state
	const [provider, setProvider] = useState<AIProvider>(
		normalizeAIProvider(forceProvider ?? defaultProvider),
	);
	const normalizedProvider = normalizeAIProvider(provider);
	const activeAgentBackend: AgentBackendProvider = isAgentBackendProvider(
		normalizedProvider,
	)
		? normalizedProvider
		: "github-copilot";

	// Board-specific state
	const [pendingCommands, setPendingCommands] = useState<BoardCommand[]>([]);
	const [pendingFlowIrCommit, setPendingFlowIrCommit] =
		useState<FlowIrCommitToken>();
	const [pendingBoardEditJob, setPendingBoardEditJob] =
		useState<BoardEditJob>();
	const pendingFlowIrCommitRef = useRef<FlowIrCommitToken | undefined>(
		undefined,
	);
	const pendingBoardEditJobRef = useRef<BoardEditJob | undefined>(undefined);
	const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
	const [currentToolCall, setCurrentToolCall] = useState<string | null>(null);
	const [flowscriptWorkspace, setFlowscriptWorkspace] = useState("");
	const [flowscriptWorkspaceStatus, setFlowscriptWorkspaceStatus] = useState<
		string | undefined
	>();
	const [inlineFlowScriptPreview, setInlineFlowScriptPreview] =
		useState<InlineFlowScriptPreviewValue | null>(null);
	const [appliedFlowScriptWorkspace, setAppliedFlowScriptWorkspace] =
		useState("");
	const [destructiveApplyRequest, setDestructiveApplyRequest] = useState<{
		flowscript: string;
		diagnostic: string;
	} | null>(null);
	const [destructiveApplyPending, setDestructiveApplyPending] = useState(false);
	const [showWorkspace, setShowWorkspace] = useState(false);
	// Session-only: auto mode resets whenever the panel remounts. It waives frontend
	// tool-approval prompts and the pending-change review gate. It never waives `ask_user`
	// or the destructive-deletion dialog.
	const [autoMode, setAutoMode] = useState(false);
	const autoModeRef = useRef(false);
	autoModeRef.current = autoMode;
	const [processEvents, setProcessEvents] = useState<FlowPilotProcessEvent[]>(
		[],
	);

	// UI-specific state
	const [pendingComponents, setPendingComponents] = useState<
		SurfaceComponent[]
	>([]);
	const [pendingCanvasSettings, setPendingCanvasSettings] = useState<
		CanvasSettings | undefined
	>();
	const [validationWarnings, setValidationWarnings] = useState<string[]>([]);

	// History state
	const [showHistory, setShowHistory] = useState(false);
	const [currentConversationId, setCurrentConversationId] = useState<
		string | undefined
	>();
	const currentMessageIdRef = useRef<string | undefined>(undefined);
	const activeCopilotRequestIdRef = useRef<string | undefined>(undefined);
	const generationMetricsRunRef = useRef<
		FlowPilotGenerationMetricsRun | undefined
	>(undefined);
	const lastBoardApplyFeedbackRef = useRef("");
	const flowIrApplyInFlightRef = useRef(false);
	const autoApplyAttemptRef = useRef<string | null>(null);
	const autoApplyComponentsAttemptRef = useRef<string | null>(null);
	const currentBoardIdRef = useRef<string | undefined>(board?.id);
	const currentBoardNodeCountRef = useRef<number | undefined>(
		flowPilotBoardNodeCount(board),
	);
	const previousBoardIdRef = useRef<string | undefined>(board?.id);
	currentBoardIdRef.current = board?.id;
	currentBoardNodeCountRef.current = flowPilotBoardNodeCount(board);
	const recordBoardApplyFailure = useCallback(
		(diagnostics: string[]) => {
			const normalizedDiagnostics = [
				...new Set(
					diagnostics
						.map((diagnostic) => diagnostic.replace(/\s+/g, " ").trim())
						.filter(Boolean),
				),
			].slice(0, 12);
			if (normalizedDiagnostics.length === 0) {
				normalizedDiagnostics.push("The queued board change failed to apply.");
			}
			setValidationWarnings(normalizedDiagnostics);

			const feedback = `${HOST_BOARD_APPLY_FAILURE_PREFIX} The live board was checked, and failed apply paths attempt a canonical refetch before this result is recorded. Inspect the current board and repair these exact failures on the next agent turn:\n${normalizedDiagnostics
				.map((diagnostic) => `- ${diagnostic}`)
				.join("\n")}`;
			if (lastBoardApplyFeedbackRef.current === feedback) return;
			lastBoardApplyFeedbackRef.current = feedback;
			setMessages((previous) => [
				...previous,
				{ role: "assistant", content: feedback },
			]);
			if (currentConversationId) {
				void addMessage(currentConversationId, {
					role: "assistant",
					content: feedback,
				}).catch((error) =>
					console.error("Failed to persist board apply feedback:", error),
				);
			}
		},
		[currentConversationId],
	);
	const recordBoardApplySuccess = useCallback(() => {
		if (!lastBoardApplyFeedbackRef.current) return;
		lastBoardApplyFeedbackRef.current = "";
		const feedback = `${HOST_BOARD_APPLY_FEEDBACK_PREFIX}\nThe previously reported queued-change failure was resolved and the reviewed board change was applied successfully.`;
		setMessages((previous) => [
			...previous,
			{ role: "assistant", content: feedback },
		]);
		if (currentConversationId) {
			void addMessage(currentConversationId, {
				role: "assistant",
				content: feedback,
			}).catch((error) =>
				console.error("Failed to persist board apply recovery:", error),
			);
		}
	}, [currentConversationId]);
	const settleGenerationReview = useCallback(
		(
			disposition: "applied" | "dismissed" | "stale" | "error",
			token?: FlowIrCommitToken,
			finalBoardNodeCount?: number,
		) => {
			const run = generationMetricsRunRef.current;
			if (!run) return;
			run.disposeReview(
				disposition,
				token,
				finalBoardNodeCount ?? currentBoardNodeCountRef.current,
			);
			if (generationMetricsRunRef.current === run) {
				generationMetricsRunRef.current = undefined;
			}
		},
		[],
	);
	const settleAuthoritativeBoardEditJob = useCallback(
		(job: BoardEditJob) => {
			if (!isSettledBoardEditJob(job)) return false;
			const ownsLocalReview =
				pendingBoardEditJobRef.current?.jobId === job.jobId ||
				pendingFlowIrCommitRef.current?.claim_id === job.token.claim_id;
			if (!ownsLocalReview) return false;

			settleGenerationReview(
				job.phase === "applied"
					? "applied"
					: job.phase === "stale"
						? "stale"
						: "dismissed",
				job.token,
				job.result?.final_board_node_count,
			);
			if (pendingBoardEditJobRef.current?.jobId === job.jobId) {
				pendingBoardEditJobRef.current = undefined;
			}
			if (pendingFlowIrCommitRef.current?.claim_id === job.token.claim_id) {
				pendingFlowIrCommitRef.current = undefined;
			}
			setPendingBoardEditJob((current) =>
				current?.jobId === job.jobId ? undefined : current,
			);
			setPendingFlowIrCommit((current) =>
				current?.claim_id === job.token.claim_id ? undefined : current,
			);
			setPendingCommands([]);
			if (job.phase === "stale") setFlowscriptWorkspaceStatus("stale");
			return true;
		},
		[settleGenerationReview],
	);

	// Refs
	const messagesEndRef = useRef<HTMLDivElement>(null);
	const scrollContainerRef = useRef<HTMLDivElement>(null);
	const imageInputRef = useRef<HTMLInputElement>(null);
	const initialPromptHandledRef = useRef(false);
	const handleSubmitRef = useRef<(() => void) | null>(null);

	// Backend context
	const backendContext = useBackend();
	const resolveFlowIrCommit = useCallback(
		async (
			token: FlowIrCommitToken,
			disposition: FlowIrCommitDisposition,
		): Promise<FlowIrCommitDispositionResult> => {
			const resolve = backendContext.boardState.flowIrCommitDisposition;
			if (!resolve) {
				return {
					status: "error",
					code: "IR_COMMIT_DISPOSITION_UNAVAILABLE",
					message:
						"This backend cannot safely resolve the retained compiled workflow review.",
				};
			}
			try {
				return await resolve.call(
					backendContext.boardState,
					token,
					disposition,
				);
			} catch (error) {
				return {
					status: "error",
					code: "IR_COMMIT_DISPOSITION_UNAVAILABLE",
					message:
						error instanceof Error
							? error.message
							: "The compiled workflow review state could not be updated.",
				};
			}
		},
		[backendContext.boardState],
	);
	const dismissFlowIrCommitWithRetry = useCallback(
		async (token: FlowIrCommitToken) => {
			let lastResult: FlowIrCommitDispositionResult = {
				status: "error",
				code: "IR_COMMIT_DISPOSITION_UNAVAILABLE",
				message: "The compiled workflow review could not be dismissed.",
			};
			for (const delayMs of FLOW_IR_DISMISS_RETRY_DELAYS_MS) {
				if (delayMs > 0) {
					await new Promise<void>((resolveDelay) =>
						setTimeout(resolveDelay, delayMs),
					);
				}
				lastResult = await resolveFlowIrCommit(token, "dismissed");
				if (
					lastResult.status === "dismissed" ||
					lastResult.code === "IR_COMMIT_TOKEN_INVALID"
				) {
					return lastResult;
				}
			}
			return lastResult;
		},
		[resolveFlowIrCommit],
	);
	const dismissPendingFlowIrCommit = useCallback(async () => {
		if (!pendingFlowIrCommit) return true;
		if (
			pendingBoardEditJob?.token.claim_id === pendingFlowIrCommit.claim_id &&
			backendContext.boardState.resolveBoardEditJob
		) {
			try {
				const resolution = await backendContext.boardState.resolveBoardEditJob(
					pendingBoardEditJob.jobId,
					false,
				);
				if (settleAuthoritativeBoardEditJob(resolution.job)) return true;
				setValidationWarnings([
					resolution.job.error ||
						"The native board-edit review could not be denied safely.",
				]);
				return false;
			} catch (error) {
				setValidationWarnings([
					error instanceof Error
						? error.message
						: "The native board-edit review could not be denied safely.",
				]);
				return false;
			}
		}
		const result = await dismissFlowIrCommitWithRetry(pendingFlowIrCommit);
		if (
			result.status === "dismissed" ||
			result.code === "IR_COMMIT_TOKEN_INVALID"
		) {
			settleGenerationReview("dismissed", pendingFlowIrCommit);
			pendingBoardEditJobRef.current = undefined;
			setPendingBoardEditJob(undefined);
			pendingFlowIrCommitRef.current = undefined;
			setPendingFlowIrCommit(undefined);
			return true;
		}
		setValidationWarnings([
			result.message ||
				"The compiled workflow review could not be dismissed; retry before leaving it.",
		]);
		return false;
	}, [
		dismissFlowIrCommitWithRetry,
		backendContext.boardState,
		pendingBoardEditJob,
		pendingFlowIrCommit,
		settleAuthoritativeBoardEditJob,
		settleGenerationReview,
	]);
	useEffect(() => {
		pendingFlowIrCommitRef.current = pendingFlowIrCommit;
	}, [pendingFlowIrCommit]);
	useEffect(() => {
		pendingBoardEditJobRef.current = pendingBoardEditJob;
	}, [pendingBoardEditJob]);
	useEffect(() => {
		const previousBoardId = previousBoardIdRef.current;
		previousBoardIdRef.current = board?.id;
		if (!previousBoardId || previousBoardId === board?.id) return;

		const activeRequestId = activeCopilotRequestIdRef.current;
		activeCopilotRequestIdRef.current = undefined;
		if (activeRequestId) {
			void backendContext.boardState
				.cancelCopilotChat?.(activeRequestId)
				.catch((error) =>
					console.error(
						"Failed to cancel FlowPilot request after board change:",
						error,
					),
				);
		}

		const pendingToken = pendingFlowIrCommitRef.current;
		const pendingJob = pendingBoardEditJobRef.current;
		pendingBoardEditJobRef.current = undefined;
		pendingFlowIrCommitRef.current = undefined;
		lastBoardApplyFeedbackRef.current = "";
		setMessages((previous) =>
			previous.filter((message) => !isHostBoardApplyFeedbackMessage(message)),
		);
		setPendingFlowIrCommit(undefined);
		setPendingBoardEditJob(undefined);
		setPendingCommands([]);
		setDestructiveApplyRequest(null);
		setFlowscriptWorkspaceStatus((status) =>
			status === "queued" ? "stale" : status,
		);
		if (!pendingToken) {
			const metricsRun = generationMetricsRunRef.current;
			metricsRun?.abandon("cancelled");
			if (generationMetricsRunRef.current === metricsRun) {
				generationMetricsRunRef.current = undefined;
			}
		}
		if (pendingToken && pendingJob?.token.claim_id !== pendingToken.claim_id) {
			void dismissFlowIrCommitWithRetry(pendingToken).then((result) => {
				if (
					result.status !== "dismissed" &&
					result.code !== "IR_COMMIT_TOKEN_INVALID"
				) {
					console.error(
						"Failed to release compiled workflow review after board change:",
						result.message,
					);
				} else {
					settleGenerationReview("stale", pendingToken);
				}
			});
		}
	}, [
		backendContext.boardState,
		board?.id,
		dismissFlowIrCommitWithRetry,
		settleGenerationReview,
	]);
	useEffect(() => {
		return () => {
			const activeRequestId = activeCopilotRequestIdRef.current;
			activeCopilotRequestIdRef.current = undefined;
			if (activeRequestId) {
				void backendContext.boardState
					.cancelCopilotChat?.(activeRequestId)
					.catch((error) =>
						console.error(
							"Failed to cancel FlowPilot request on unmount:",
							error,
						),
					);
			}
			const pendingToken = pendingFlowIrCommitRef.current;
			const pendingJob = pendingBoardEditJobRef.current;
			if (
				pendingToken &&
				pendingJob?.token.claim_id !== pendingToken.claim_id
			) {
				void dismissFlowIrCommitWithRetry(pendingToken).then((result) => {
					if (
						result.status !== "dismissed" &&
						result.code !== "IR_COMMIT_TOKEN_INVALID"
					) {
						console.error(
							"Failed to release compiled workflow review on unmount:",
							result.message,
						);
					} else {
						settleGenerationReview("stale", pendingToken);
					}
				});
			} else if (!pendingJob) {
				const metricsRun = generationMetricsRunRef.current;
				metricsRun?.abandon("cancelled");
				if (generationMetricsRunRef.current === metricsRun) {
					generationMetricsRunRef.current = undefined;
				}
			}
		};
	}, [
		backendContext.boardState,
		dismissFlowIrCommitWithRetry,
		settleGenerationReview,
	]);
	const activeAppId = appId ?? runContext?.app_id;
	useEffect(() => {
		const listJobs = backendContext.boardState.listBoardEditJobs;
		if (!listJobs || !activeAppId || !board?.id) return;
		let cancelled = false;
		let pollTimer: ReturnType<typeof setTimeout> | undefined;
		const pollJobs = async () => {
			try {
				const jobs = await listJobs.call(
					backendContext.boardState,
					activeAppId,
					board.id,
					false,
				);
				if (cancelled) return;
				const direct = jobs
					.filter(
						(job) =>
							isDirectFlowPilotBoardEditJob(job) &&
							(job.phase === "awaiting_approval" ||
								job.phase === "failed" ||
								job.phase === "applied_pending_delivery"),
					)
					.toSorted(
						(left, right) =>
							left.createdAtMs - right.createdAtMs ||
							left.jobId.localeCompare(right.jobId),
					);
				const current = pendingBoardEditJobRef.current;
				let pending = current
					? direct.find((job) => job.jobId === current.jobId)
					: undefined;
				if (!pending && !pendingFlowIrCommitRef.current) pending = direct[0];
				if (!pending) return;

				if (
					pending.phase === "applied_pending_delivery" &&
					onApplyFlowIrCommit
				) {
					const delivery = await deliverBoardEditJobReceipt({
						boardState: backendContext.boardState,
						job: pending,
						replayReceipt: onApplyFlowIrCommit,
						historyMode: "invalidate",
					});
					if (cancelled) return;
					if (
						delivery.status === "delivered" ||
						delivery.status === "settled"
					) {
						settleAuthoritativeBoardEditJob(delivery.job);
						return;
					}
					pending = delivery.job;
				}

				const newlyAdopted = current?.jobId !== pending.jobId;
				pendingBoardEditJobRef.current = pending;
				setPendingBoardEditJob(pending);
				pendingFlowIrCommitRef.current = pending.token;
				setPendingFlowIrCommit(pending.token);
				setPendingCommands(
					(pending.result?.board_commands ?? []) as BoardCommand[],
				);
				if (newlyAdopted) {
					setValidationWarnings((warnings) => [
						...warnings,
						...(pending.error ? [pending.error] : []),
						`Recovered a compiled workflow review with ${pending.review.commandCount} exact board command(s).`,
					]);
				}
			} catch (error) {
				console.warn("Failed to rehydrate FlowPilot board-edit review:", error);
			} finally {
				if (!cancelled) {
					pollTimer = setTimeout(pollJobs, BOARD_EDIT_JOB_POLL_INTERVAL_MS);
				}
			}
		};
		void pollJobs();
		return () => {
			cancelled = true;
			if (pollTimer) clearTimeout(pollTimer);
		};
	}, [
		activeAppId,
		backendContext.boardState,
		board?.id,
		onApplyFlowIrCommit,
		settleAuthoritativeBoardEditJob,
	]);
	const executeRuntimeTool = useFrontendRuntimeToolExecutor({
		defaultAppId: activeAppId,
		defaultBoardId: board?.id,
	});
	const approvedFrontendToolKeysRef = useRef<Set<string>>(new Set());
	const frontendToolRequestGuardRef = useRef(new FrontendToolRequestGuard());
	const frontendToolDialogResolverRef = useRef<((value: any) => void) | null>(
		null,
	);
	const frontendToolDialogRef = useRef<FrontendToolDialogState | null>(null);
	const frontendToolDialogQueueRef = useRef<FrontendToolQueuedDialog[]>([]);
	const [frontendToolDialog, setFrontendToolDialog] =
		useState<FrontendToolDialogState | null>(null);

	// Agent backend hook
	const copilotSDK = useCopilotSDK(activeAgentBackend);

	// Fetch user profile
	const profile = useInvoke(
		backendContext.userState.getSettingsProfile,
		backendContext.userState,
		[],
		true,
	);

	// Fetch available models (bits)
	const foundBits = useInvoke(
		backendContext.bitState.searchBits,
		backendContext.bitState,
		[{ bit_types: [IBitTypes.Llm, IBitTypes.Vlm] }],
		!!profile.data,
		[profile.data?.hub_profile.id],
	);

	// User-owned custom bits are always selectable, independent of profile
	// membership.
	const customBits = useInvoke(
		backendContext.bitState.listCustomBits,
		backendContext.bitState,
		[],
		!!profile.data,
		[profile.data?.hub_profile.id],
	);

	// Filter bits models to those in the user's profile, dropping local-only
	// models on hosts that cannot run llama.cpp (e.g. iOS).
	const canHostLlamaCPP = backendContext.capabilities().canHostLlamaCPP;
	const bitsModels = useMemo(() => {
		if (!foundBits.data || !profile.data?.hub_profile.bits) return [];
		const profileBitIds = new Set(profile.data.hub_profile.bits);
		const profileModels = foundBits.data.filter((model) =>
			profileBitIds.has(`${model.hub}:${model.id}`),
		);
		const seen = new Set(profileModels.map((model) => model.id));
		const ownModels = (customBits.data ?? []).filter(
			(model) =>
				!seen.has(model.id) &&
				(model.type === IBitTypes.Llm || model.type === IBitTypes.Vlm),
		);
		return filterHostableLlmModels(
			[...ownModels, ...profileModels],
			canHostLlamaCPP,
		);
	}, [
		foundBits.data,
		customBits.data,
		profile.data?.hub_profile.bits,
		canHostLlamaCPP,
	]);

	const openFrontendToolDialog = useCallback(
		(dialog: FrontendToolDialogState, resolve: (value: any) => void) => {
			if (frontendToolDialogResolverRef.current) {
				frontendToolDialogQueueRef.current.push({ dialog, resolve });
				return;
			}
			frontendToolDialogResolverRef.current = resolve;
			frontendToolDialogRef.current = dialog;
			setFrontendToolDialog(dialog);
		},
		[],
	);

	const requestFrontendToolApproval = useCallback(
		(
			request: FrontendToolRequest,
		): Promise<{ approved: boolean; remember: boolean }> => {
			const approval = request.approval;
			const sessionKey =
				approval?.sessionKey ||
				`${request.toolName}:${approval?.kind ?? "none"}`;
			// Auto mode is read through a ref so toggling it never changes this callback's
			// identity: the Tauri bridge listener would otherwise tear down and cancel every
			// in-flight request. `remember: false` keeps auto-approvals out of the session
			// allowlist, so turning auto mode off restores prompting.
			if (
				autoModeRef.current ||
				approval?.kind === "none" ||
				shouldSkipUnavailableCreateTableApproval(
					request.toolName,
					request.arguments,
					activeAppId,
				) ||
				approvedFrontendToolKeysRef.current.has(sessionKey)
			) {
				return Promise.resolve({ approved: true, remember: false });
			}

			return new Promise((resolve) => {
				openFrontendToolDialog(
					{
						type: "approval",
						request,
						remember: false,
					},
					resolve,
				);
			});
		},
		[activeAppId, openFrontendToolDialog],
	);

	const requestFrontendUserInput = useCallback(
		(request: FrontendToolRequest): Promise<unknown> => {
			const args = request.arguments;
			const mode =
				getArgString(args, "mode") === "multiple_choice"
					? "multiple_choice"
					: getArgString(args, "mode") === "single_choice"
						? "single_choice"
						: "freeform";
			const choices = Array.isArray(args.choices)
				? (args.choices as FrontendToolChoice[])
				: [];
			const defaultValue = args.default_value ?? args.defaultValue;
			const selected = new Set<number>();

			if (mode !== "freeform" && choices.length > 0) {
				const defaultIndex = choices.findIndex(
					(choice) =>
						choice.value === defaultValue ||
						choice.label === defaultValue ||
						(defaultValue === undefined && selected.size === 0),
				);
				selected.add(defaultIndex >= 0 ? defaultIndex : 0);
			}

			return new Promise((resolve) => {
				openFrontendToolDialog(
					{
						type: "ask",
						request,
						mode,
						answer:
							typeof defaultValue === "string"
								? defaultValue
								: defaultValue === undefined
									? ""
									: JSON.stringify(defaultValue),
						selected,
					},
					resolve,
				);
			});
		},
		[openFrontendToolDialog],
	);

	const cancelFrontendToolDialogs = useCallback((requestId: string) => {
		const activeDialog = frontendToolDialogRef.current;
		if (activeDialog?.request.requestId === requestId) {
			const resolver = frontendToolDialogResolverRef.current;
			frontendToolDialogResolverRef.current = null;
			frontendToolDialogRef.current = null;
			resolver?.(
				activeDialog.type === "approval"
					? { approved: false, remember: false }
					: null,
			);
			setFrontendToolDialog(null);
		}

		const retained: FrontendToolQueuedDialog[] = [];
		for (const queued of frontendToolDialogQueueRef.current) {
			if (queued.dialog.request.requestId !== requestId) {
				retained.push(queued);
				continue;
			}
			queued.resolve(
				queued.dialog.type === "approval"
					? { approved: false, remember: false }
					: null,
			);
		}
		frontendToolDialogQueueRef.current = retained;

		if (!frontendToolDialogResolverRef.current) {
			const next = frontendToolDialogQueueRef.current.shift();
			if (next) {
				frontendToolDialogResolverRef.current = next.resolve;
				frontendToolDialogRef.current = next.dialog;
				setFrontendToolDialog(next.dialog);
			}
		}
	}, []);

	const executeFrontendToolRequest = useCallback(
		async (
			request: FrontendToolRequest,
			lease: FrontendToolRequestLease,
		): Promise<FrontendToolResponse> => {
			try {
				lease.assertActive("approval handling");
				if (request.toolName === "ask_user") {
					const answer = await requestFrontendUserInput(request);
					lease.assertActive("question response");
					return {
						requestId: request.requestId,
						approved: true,
						result: { status: "ok", answer },
					};
				}

				const approval = await requestFrontendToolApproval(request);
				lease.assertActive("approval response");
				if (!approval.approved) {
					return {
						requestId: request.requestId,
						approved: false,
						error: "User denied the request.",
					};
				}
				const sessionKey =
					request.approval?.sessionKey ||
					`${request.toolName}:${request.approval?.kind ?? "none"}`;
				if (approval.remember && request.approval?.kind !== "none") {
					lease.assertActive("approval persistence");
					approvedFrontendToolKeysRef.current.add(sessionKey);
				}

				lease.assertActive("tool execution");
				let result: unknown;
				switch (request.toolName) {
					case "database_tool":
					case "storage_tool":
					case "ui_inspect":
					case "execute_event":
					case "execute_node":
					case "query_execution_logs":
						result = await executeRuntimeTool(
							request.toolName,
							request.arguments,
						);
						break;
					default:
						throw new Error(`Unsupported frontend tool '${request.toolName}'.`);
				}
				lease.assertActive("tool result publication");

				return {
					requestId: request.requestId,
					approved: true,
					result,
				};
			} catch (error) {
				return {
					requestId: request.requestId,
					approved: true,
					error: error instanceof Error ? error.message : String(error),
				};
			}
		},
		[executeRuntimeTool, requestFrontendToolApproval, requestFrontendUserInput],
	);
	const executeFrontendToolRequestRef = useRef(executeFrontendToolRequest);

	useEffect(() => {
		executeFrontendToolRequestRef.current = executeFrontendToolRequest;
	}, [executeFrontendToolRequest]);

	useEffect(() => {
		if (!isTauriRuntime()) return;

		let disposed = false;
		let unlisten: (() => void) | undefined;

		async function installListener() {
			try {
				const [eventApi, coreApi] = await Promise.all([
					importTauriEvent(),
					importTauriCore(),
				]);

				const [stop, stopCancellation] = await Promise.all([
					eventApi.listen<FrontendToolRequest>(
						FLOWPILOT_FRONTEND_TOOL_EVENT,
						async (event) => {
							if (disposed) return;
							const request = event.payload;
							if (!request?.requestId || !request.toolName) return;
							const lease = frontendToolRequestGuardRef.current.begin({
								requestId: request.requestId,
								deadlineAtMs: resolveFrontendToolExecutionDeadline({
									toolName: request.toolName,
									backendDeadlineAtMs:
										request.deadlineAtMs ?? request.deadline_at_ms,
								}),
								onInvalidated: () =>
									cancelFrontendToolDialogs(request.requestId),
							});
							// Keep the original immutable generation until it settles; a duplicate
							// event must never revive a cancelled request id.
							if (!lease) return;
							try {
								const response = await executeFrontendToolRequestRef.current(
									request,
									lease,
								);
								if (disposed || !lease.isActive()) return;
								lease.assertActive("frontend response delivery");
								try {
									await coreApi.invoke("flowpilot_frontend_tool_result", {
										response,
									});
								} catch (error) {
									if (!disposed) {
										console.warn(
											"Failed to return FlowPilot frontend tool result:",
											error,
										);
									}
								}
							} finally {
								lease.settle();
							}
						},
					),
					eventApi.listen<{ requestId?: string }>(
						FLOWPILOT_FRONTEND_TOOL_CANCEL_EVENT,
						(event) => {
							const requestId = event.payload?.requestId;
							if (!requestId) return;
							frontendToolRequestGuardRef.current.cancel(
								requestId,
								"cancelled",
							);
						},
					),
				]);

				if (disposed) {
					stop();
					stopCancellation();
				} else {
					unlisten = () => {
						stop();
						stopCancellation();
					};
				}
			} catch (error) {
				console.warn(
					"Failed to install FlowPilot frontend tool bridge:",
					error,
				);
			}
		}

		void installListener();

		return () => {
			disposed = true;
			unlisten?.();
			frontendToolRequestGuardRef.current.cancelAll("unmounted");
		};
	}, [cancelFrontendToolDialogs]);

	const resolveFrontendToolDialog = useCallback(
		(requestId: string, value: unknown) => {
			// A click queued from a dialog that was cancelled must not answer the next dialog
			// rendered in the same slot.
			if (frontendToolDialogRef.current?.request.requestId !== requestId)
				return;
			const resolver = frontendToolDialogResolverRef.current;
			frontendToolDialogResolverRef.current = null;
			resolver?.(value);

			const next = frontendToolDialogQueueRef.current.shift();
			if (next) {
				frontendToolDialogResolverRef.current = next.resolve;
				frontendToolDialogRef.current = next.dialog;
				setFrontendToolDialog(next.dialog);
			} else {
				frontendToolDialogRef.current = null;
				setFrontendToolDialog(null);
			}
		},
		[],
	);

	// Turning auto mode on mid-run must settle approval dialogs that are already on screen or
	// queued; their promises were captured before the flip and would otherwise block until the
	// backend deadline. `ask_user` dialogs stay up — auto mode never answers questions.
	const flushPendingToolApprovals = useCallback(() => {
		if (frontendToolDialogRef.current?.type === "approval") {
			const resolver = frontendToolDialogResolverRef.current;
			frontendToolDialogResolverRef.current = null;
			frontendToolDialogRef.current = null;
			resolver?.({ approved: true, remember: false });
			setFrontendToolDialog(null);
		}

		const retained: FrontendToolQueuedDialog[] = [];
		for (const queued of frontendToolDialogQueueRef.current) {
			if (queued.dialog.type !== "approval") {
				retained.push(queued);
				continue;
			}
			queued.resolve({ approved: true, remember: false });
		}
		frontendToolDialogQueueRef.current = retained;

		if (!frontendToolDialogResolverRef.current) {
			const next = frontendToolDialogQueueRef.current.shift();
			if (next) {
				frontendToolDialogResolverRef.current = next.resolve;
				frontendToolDialogRef.current = next.dialog;
				setFrontendToolDialog(next.dialog);
			}
		}
	}, []);

	const handleToggleAutoMode = useCallback(() => {
		const next = !autoModeRef.current;
		autoModeRef.current = next;
		setAutoMode(next);
		if (next) flushPendingToolApprovals();
	}, [flushPendingToolApprovals]);

	useEffect(
		() => () => {
			// Navigation can unmount the board while the native agent is awaiting approval/input.
			// Settle every promise so the bridge can unwind instead of waiting until its timeout.
			const active = frontendToolDialogResolverRef.current;
			const activeDialog = frontendToolDialogRef.current;
			frontendToolDialogResolverRef.current = null;
			frontendToolDialogRef.current = null;
			active?.(
				activeDialog?.type === "approval"
					? { approved: false, remember: false }
					: null,
			);
			for (const queued of frontendToolDialogQueueRef.current.splice(0)) {
				queued.resolve(
					queued.dialog.type === "approval"
						? { approved: false, remember: false }
						: null,
				);
			}
		},
		[],
	);

	// Get current models based on provider
	const currentModels = useMemo(() => {
		if (isAgentBackendProvider(normalizedProvider)) {
			return copilotSDK.models;
		}
		return bitsModels;
	}, [normalizedProvider, copilotSDK.models, bitsModels]);
	const previousModelProviderRef = useRef(normalizedProvider);
	const selectedAgentModel = isAgentBackendProvider(normalizedProvider)
		? copilotSDK.models.find((model) => model.id === selectedModelId)
		: undefined;

	// Set default model when models are loaded or provider changes
	useEffect(() => {
		if (currentModels.length === 0) return;

		const providerChanged =
			previousModelProviderRef.current !== normalizedProvider;
		previousModelProviderRef.current = normalizedProvider;

		// Check if current selection is valid for current provider
		const isCurrentValid = currentModels.some((m) => m.id === selectedModelId);
		if (!providerChanged && isCurrentValid) return;

		// Select a default model
		if (isAgentBackendProvider(normalizedProvider)) {
			let preferredModel = currentModels[0];
			if (normalizedProvider === "codex") {
				preferredModel =
					currentModels.find((m) => m.id === "default") || currentModels[0];
			} else if (normalizedProvider === "claude-code") {
				// Honor the CLI's recommended "default" entry (from dynamic
				// discovery) before falling back to Sonnet, matching Codex.
				preferredModel =
					currentModels.find((m) => m.id === "default") ||
					currentModels.find((m) => m.id.includes("sonnet")) ||
					currentModels[0];
			} else {
				preferredModel =
					currentModels.find((m) => m.id.includes("claude")) ||
					currentModels.find((m) => m.id.includes("gpt-4")) ||
					currentModels[0];
			}
			setSelectedModelId(preferredModel?.id || "");
		} else {
			// Bits provider - existing logic
			const hostedModel = bitsModels.find(
				(m) => m.parameters?.provider?.provider_name === "Hosted",
			);
			const gpt4o = bitsModels.find((m) => m.id.includes("gpt-4o"));
			const defaultModel = hostedModel || gpt4o || bitsModels[0];
			setSelectedModelId(defaultModel?.id || "");
		}
	}, [currentModels, selectedModelId, normalizedProvider, bitsModels]);

	// Effort capabilities belong to a specific provider/model pair. Keep an
	// explicit choice only while the live catalog still advertises it; an empty
	// value means the backend remains in control of its default.
	useEffect(() => {
		const efforts = selectedAgentModel?.supportedReasoningEfforts ?? [];
		if (
			selectedReasoningEffort &&
			!efforts.some((effort) => effort.id === selectedReasoningEffort)
		) {
			setSelectedReasoningEffort("");
		}
	}, [selectedAgentModel, selectedReasoningEffort]);

	// Copilot connection handlers
	const handleStartCopilot = useCallback(
		async (backend?: AgentBackendProvider, serverUrl?: string) => {
			await copilotSDK.start({
				backend: backend ?? activeAgentBackend,
				useStdio: !serverUrl,
				serverUrl,
			});
		},
		[activeAgentBackend, copilotSDK],
	);

	const handleStopCopilot = useCallback(async () => {
		const activeRequestId = activeCopilotRequestIdRef.current;
		activeCopilotRequestIdRef.current = undefined;
		if (activeRequestId) {
			try {
				await backendContext.boardState.cancelCopilotChat?.(activeRequestId);
			} catch (error) {
				console.error("Failed to cancel active FlowPilot request:", error);
			}
		}
		await copilotSDK.stop();
		const metricsRun = generationMetricsRunRef.current;
		metricsRun?.abandon("cancelled");
		if (generationMetricsRunRef.current === metricsRun) {
			generationMetricsRunRef.current = undefined;
		}
		setProvider("bits");
	}, [backendContext.boardState, copilotSDK]);

	// Scroll handling
	const scrollToBottom = useCallback(
		(force = false) => {
			if (force || !userScrolledUp) {
				messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
			}
		},
		[userScrolledUp],
	);

	const handleScroll = useCallback(() => {
		const container = scrollContainerRef.current;
		if (!container) return;
		const { scrollTop, scrollHeight, clientHeight } = container;
		const isAtBottom = scrollHeight - scrollTop - clientHeight < 100;
		setUserScrolledUp(!isAtBottom);
	}, []);

	useEffect(() => {
		if (!userScrolledUp) {
			scrollToBottom();
		}
	}, [messages, userScrolledUp, scrollToBottom]);

	// New chat handler
	const handleNewChat = useCallback(async () => {
		if (!(await dismissPendingFlowIrCommit())) return;
		settleGenerationReview("dismissed", pendingFlowIrCommit);
		setMessages([]);
		setPlanSteps([]);
		setInput("");
		setAttachedImages([]);
		setPendingCommands([]);
		setPendingComponents([]);
		setValidationWarnings([]);
		setSuggestions([]);
		setFlowscriptWorkspace("");
		setFlowscriptWorkspaceStatus(undefined);
		setInlineFlowScriptPreview(null);
		setAppliedFlowScriptWorkspace("");
		setDestructiveApplyRequest(null);
		setDestructiveApplyPending(false);
		setShowWorkspace(false);
		setProcessEvents([]);
		setCurrentConversationId(undefined);
		currentMessageIdRef.current = undefined;
		// A regenerated review can be byte-identical to the last one; clearing the stamps keeps
		// auto mode from mistaking it for an apply it already attempted.
		autoApplyAttemptRef.current = null;
		autoApplyComponentsAttemptRef.current = null;
		lastBoardApplyFeedbackRef.current = "";
		setShowHistory(false);
	}, [dismissPendingFlowIrCommit, pendingFlowIrCommit, settleGenerationReview]);

	// Select conversation from history
	const handleSelectConversation = useCallback(
		async (conversation: IFlowPilotConversation) => {
			if (!(await dismissPendingFlowIrCommit())) return;
			settleGenerationReview("dismissed", pendingFlowIrCommit);
			try {
				const storedMessages = await getMessages(conversation.id);
				const loadedMessages: CopilotMessage[] = storedMessages.map((m) => ({
					role: m.role as "user" | "assistant",
					content: m.content,
					images: m.images?.map((image) => ({
						data: image.data,
						mediaType: image.mediaType,
						preview: `data:${image.mediaType};base64,${image.data}`,
					})),
					contextNodeIds: m.contextNodeIds,
					appliedComponents: m.appliedComponents,
					executedCommands: m.executedCommands,
					flowscriptWorkspace: m.flowscriptWorkspace,
					processEvents: m.processEvents,
					planSteps: m.planSteps,
				}));
				const latestWorkspace = [...loadedMessages]
					.reverse()
					.find((message) => message.flowscriptWorkspace)?.flowscriptWorkspace;
				setMessages(loadedMessages);
				setFlowscriptWorkspace(latestWorkspace ?? "");
				setFlowscriptWorkspaceStatus(undefined);
				setInlineFlowScriptPreview(null);
				setAppliedFlowScriptWorkspace(latestWorkspace ?? "");
				setDestructiveApplyRequest(null);
				setDestructiveApplyPending(false);
				setShowWorkspace(Boolean(latestWorkspace));
				setCurrentConversationId(conversation.id);
				setPlanSteps([]);
				setPendingCommands([]);
				setPendingComponents([]);
				setValidationWarnings([]);
				setProcessEvents([]);
				currentMessageIdRef.current = undefined;
				autoApplyAttemptRef.current = null;
				autoApplyComponentsAttemptRef.current = null;
				lastBoardApplyFeedbackRef.current =
					latestUnresolvedBoardApplyFeedback(loadedMessages);
				setShowHistory(false);
			} catch (err) {
				console.error("Failed to load conversation:", err);
			}
		},
		[dismissPendingFlowIrCommit, pendingFlowIrCommit, settleGenerationReview],
	);

	// Image handling
	const handleImageSelect = useCallback(
		(e: React.ChangeEvent<HTMLInputElement>) => {
			const files = e.target.files;
			if (!files) return;

			Array.from(files).forEach((file) => {
				if (!canAttachImage(file)) {
					console.warn(
						`Skipped unsupported or oversized FlowPilot image: ${file.name}`,
					);
					return;
				}

				const reader = new FileReader();
				reader.onload = (event) => {
					const dataUrl = event.target?.result as string;
					if (!dataUrl) return;

					const base64Data = dataUrl.split(",")[1];
					if (
						!base64Data ||
						base64ByteLength(base64Data) > MAX_ATTACHED_IMAGE_BYTES
					) {
						return;
					}
					setAttachedImages((prev) => {
						if (prev.length >= MAX_ATTACHED_IMAGES) return prev;
						return [
							...prev,
							{
								data: base64Data,
								mediaType: file.type,
								preview: dataUrl,
							},
						];
					});
				};
				reader.readAsDataURL(file);
			});

			if (imageInputRef.current) {
				imageInputRef.current.value = "";
			}
		},
		[],
	);

	const handleRemoveImage = useCallback((index: number) => {
		setAttachedImages((prev) => prev.filter((_, i) => i !== index));
	}, []);

	const handlePaste = useCallback((e: React.ClipboardEvent) => {
		const items = e.clipboardData?.items;
		if (!items) return;

		for (const item of Array.from(items)) {
			if (item.type.startsWith("image/")) {
				e.preventDefault();
				const file = item.getAsFile();
				if (!file) continue;
				if (!canAttachImage(file)) {
					console.warn(
						"Skipped unsupported or oversized pasted FlowPilot image",
					);
					continue;
				}

				const reader = new FileReader();
				reader.onload = (event) => {
					const dataUrl = event.target?.result as string;
					if (!dataUrl) return;

					const base64Data = dataUrl.split(",")[1];
					if (
						!base64Data ||
						base64ByteLength(base64Data) > MAX_ATTACHED_IMAGE_BYTES
					) {
						return;
					}
					setAttachedImages((prev) => {
						if (prev.length >= MAX_ATTACHED_IMAGES) return prev;
						return [
							...prev,
							{
								data: base64Data,
								mediaType: file.type,
								preview: dataUrl,
							},
						];
					});
				};
				reader.readAsDataURL(file);
			}
		}
	}, []);

	const recordExecutedBoardCommands = useCallback(
		(appliedBoardCommands: BoardCommand[]) => {
			const lastAssistantMessage = [...messages]
				.reverse()
				.find(
					(message) =>
						message.role === "assistant" &&
						!isHostBoardApplyFeedbackMessage(message),
				);
			const nextExecutedCommands = [
				...(lastAssistantMessage?.executedCommands ?? []),
				...appliedBoardCommands,
			];
			setMessages((prev) => {
				const newMessages = [...prev];
				for (let i = newMessages.length - 1; i >= 0; i--) {
					if (
						newMessages[i].role === "assistant" &&
						!isHostBoardApplyFeedbackMessage(newMessages[i])
					) {
						const existingCommands = newMessages[i].executedCommands || [];
						newMessages[i] = {
							...newMessages[i],
							executedCommands: [...existingCommands, ...appliedBoardCommands],
						};
						break;
					}
				}
				return newMessages;
			});
			if (currentMessageIdRef.current) {
				void updateMessage(currentMessageIdRef.current, {
					executedCommands: nextExecutedCommands,
				});
			}
		},
		[messages],
	);

	const preflightPendingFlowIrCommit = useCallback(async () => {
		if (!pendingFlowIrCommit) return true;
		if (
			pendingBoardEditJob?.token.claim_id === pendingFlowIrCommit.claim_id &&
			backendContext.boardState.getBoardEditJob
		) {
			try {
				const latestJob = await backendContext.boardState.getBoardEditJob(
					pendingBoardEditJob.jobId,
				);
				if (latestJob) {
					if (settleAuthoritativeBoardEditJob(latestJob)) return false;
					if (pendingBoardEditJobRef.current?.jobId === latestJob.jobId) {
						pendingBoardEditJobRef.current = latestJob;
						setPendingBoardEditJob(latestJob);
					}
					// Native mutation already passed its CAS preflight. The only remaining work is
					// idempotent renderer receipt delivery under the job's delivery lease.
					if (latestJob.phase === "applied_pending_delivery") return true;
					// The durable native job owns the exact command batch and rechecks its original
					// board fingerprint inside the board write lock. This remains authoritative after
					// a process restart where the ephemeral compiler-store claim no longer exists.
					if (
						latestJob.phase === "awaiting_approval" ||
						latestJob.phase === "failed"
					) {
						return true;
					}
				}
			} catch (error) {
				console.warn(
					"Failed to refresh the native board-edit review before apply:",
					error,
				);
			}
		}
		const result = await resolveFlowIrCommit(pendingFlowIrCommit, "preflight");
		if (result.status === "current") return true;

		recordBoardApplyFailure([
			result.message ||
				"The board changed after this compiled workflow was generated. Regenerate it from the current board before applying.",
		]);
		if (result.code === "IR_COMMIT_REVIEW_STALE") {
			const dismissed = await resolveFlowIrCommit(
				pendingFlowIrCommit,
				"dismissed",
			);
			if (dismissed.status === "dismissed") {
				settleGenerationReview("stale", pendingFlowIrCommit);
				pendingFlowIrCommitRef.current = undefined;
				setPendingFlowIrCommit(undefined);
				setPendingCommands([]);
				setFlowscriptWorkspaceStatus("stale");
			}
		}
		return false;
	}, [
		backendContext.boardState,
		pendingBoardEditJob,
		pendingFlowIrCommit,
		recordBoardApplyFailure,
		resolveFlowIrCommit,
		settleAuthoritativeBoardEditJob,
		settleGenerationReview,
	]);

	// Board mode handlers
	const executePendingCommands = useCallback(async () => {
		const hasRetainedCompiledBatch = Boolean(pendingFlowIrCommit);
		if (
			pendingFlowIrCommit &&
			pendingFlowIrCommit.board_id !== currentBoardIdRef.current
		) {
			setValidationWarnings([
				"This compiled workflow review belongs to a different board and cannot be applied here. It has been dismissed; regenerate against the board currently open.",
			]);
			await dismissPendingFlowIrCommit();
			setPendingCommands([]);
			setFlowscriptWorkspaceStatus("stale");
			return;
		}
		if (hasRetainedCompiledBatch && flowIrApplyInFlightRef.current) return;
		const shouldApplyFlowScript =
			!hasRetainedCompiledBatch &&
			Boolean(onApplyFlowScript) &&
			isFlowScriptWorkspaceApplicable({
				source: flowscriptWorkspace,
				status: flowscriptWorkspaceStatus,
			}) &&
			flowscriptWorkspace !== appliedFlowScriptWorkspace;
		if (
			hasRetainedCompiledBatch ||
			shouldApplyFlowScript ||
			(onExecuteCommands && pendingCommands.length > 0)
		) {
			if (hasRetainedCompiledBatch && !(await preflightPendingFlowIrCommit()))
				return;
			let appliedBoardCommands: BoardCommand[] = pendingCommands;
			let finalBoardNodeCount: number | undefined;
			try {
				let applyResult: unknown;
				if (hasRetainedCompiledBatch) {
					const token = pendingFlowIrCommit;
					if (!token || !onApplyFlowIrCommit) {
						recordBoardApplyFailure([
							"This backend cannot atomically apply the retained compiled workflow batch. It was not re-reconciled or partially executed; dismiss it and regenerate on a supported host.",
						]);
						return;
					}
					flowIrApplyInFlightRef.current = true;
					let compiledResult: Awaited<ReturnType<typeof onApplyFlowIrCommit>>;
					try {
						if (
							pendingBoardEditJob?.token.claim_id === token.claim_id &&
							backendContext.boardState.resolveBoardEditJob
						) {
							const resolution =
								await backendContext.boardState.resolveBoardEditJob(
									pendingBoardEditJob.jobId,
									true,
								);
							if (isSettledBoardEditJob(resolution.job)) {
								if (resolution.job.phase !== "applied") {
									recordBoardApplyFailure([
										resolution.job.error ||
											resolution.job.result?.message ||
											"The native board-edit job settled without applying.",
									]);
								}
								settleAuthoritativeBoardEditJob(resolution.job);
								return;
							}
							if (resolution.job.phase !== "applied_pending_delivery") {
								recordBoardApplyFailure([
									resolution.job.error ||
										resolution.job.result?.message ||
										"The native board-edit job did not reach receipt delivery.",
								]);
								return;
							}
							const delivery = await deliverBoardEditJobReceipt({
								boardState: backendContext.boardState,
								job: resolution.job,
								replayReceipt: onApplyFlowIrCommit,
								historyMode: boardEditJobResolutionHistoryMode(resolution),
							});
							if (delivery.status === "settled") {
								settleAuthoritativeBoardEditJob(delivery.job);
								return;
							}
							if (delivery.status === "busy") {
								setValidationWarnings([delivery.message]);
								return;
							}
							if (delivery.status !== "delivered") {
								recordBoardApplyFailure([
									delivery.message,
									...(delivery.status === "replay_failed"
										? delivery.receipt.diagnostics
										: []),
								]);
								return;
							}
							compiledResult = delivery.receipt;
						} else {
							compiledResult = await onApplyFlowIrCommit(token);
						}
					} finally {
						flowIrApplyInFlightRef.current = false;
					}
					if (pendingFlowIrCommitRef.current?.claim_id !== token.claim_id) {
						return;
					}
					applyResult = compiledResult;
					appliedBoardCommands = compiledResult.board_commands;
					finalBoardNodeCount = compiledResult.final_board_node_count;
					if (compiledResult.status !== "applied") {
						recordBoardApplyFailure([
							...(compiledResult.code ? [`${compiledResult.code}`] : []),
							compiledResult.message ||
								"The live board no longer matches this compiled workflow review.",
							...compiledResult.diagnostics,
						]);
						if (compiledResult.status === "stale") {
							settleGenerationReview("stale", token);
							await dismissPendingFlowIrCommit();
							setPendingCommands([]);
							setFlowscriptWorkspaceStatus("stale");
						} else {
							settleGenerationReview("error", token);
						}
						return;
					}
					pendingFlowIrCommitRef.current = undefined;
					setPendingFlowIrCommit(undefined);
					pendingBoardEditJobRef.current = undefined;
					setPendingBoardEditJob(undefined);
				} else if (shouldApplyFlowScript && onApplyFlowScript) {
					if (backendContext.boardState.createBoardEditJob) {
						recordBoardApplyFailure([
							"This desktop review contains only legacy FlowScript source and has no durable compiled receipt. It was not applied; regenerate the review so it can use crash-safe atomic delivery.",
						]);
						return;
					}
					applyResult = await onApplyFlowScript(flowscriptWorkspace, {
						suppressBlockedToast: true,
					});
					if (!applyResult) return;

					appliedBoardCommands = applyResultBoardCommands(applyResult);
				} else if (onExecuteCommands) {
					await onExecuteCommands(pendingCommands);
				}
				if (
					finalBoardNodeCount === undefined &&
					applyResult &&
					typeof applyResult === "object"
				) {
					const reportedCount = (
						applyResult as { final_board_node_count?: unknown }
					).final_board_node_count;
					if (
						Number.isSafeInteger(reportedCount) &&
						(reportedCount as number) >= 0
					) {
						finalBoardNodeCount = reportedCount as number;
					}
				}
				const diagnostics = applyResultDiagnostics(applyResult);
				if (
					applyResultCommandCount(applyResult) === 0 &&
					diagnostics.length > 0
				) {
					const destructiveDiagnostic =
						destructiveFlowScriptDiagnostic(diagnostics);
					if (shouldApplyFlowScript && destructiveDiagnostic) {
						setDestructiveApplyRequest({
							flowscript: flowscriptWorkspace,
							diagnostic: destructiveDiagnostic,
						});
						return;
					}
					recordBoardApplyFailure(diagnostics);
					setFlowscriptWorkspaceStatus("validation_errors");
					settleGenerationReview("error", pendingFlowIrCommit);
					return;
				}
				if (shouldApplyFlowScript || hasRetainedCompiledBatch) {
					setAppliedFlowScriptWorkspace(flowscriptWorkspace);
					setFlowscriptWorkspaceStatus("applied");
				}
			} catch (error) {
				settleGenerationReview("error", pendingFlowIrCommit);
				recordBoardApplyFailure(flowPilotCommandApplyDiagnostics(error));
				console.error("Failed to apply FlowPilot commands:", error);
				return;
			}
			settleGenerationReview(
				"applied",
				pendingFlowIrCommit,
				finalBoardNodeCount,
			);
			recordBoardApplySuccess();
			recordExecutedBoardCommands(appliedBoardCommands);
			setPendingCommands([]);
			setDestructiveApplyRequest(null);
		}
	}, [
		appliedFlowScriptWorkspace,
		flowscriptWorkspace,
		flowscriptWorkspaceStatus,
		onApplyFlowScript,
		onExecuteCommands,
		pendingCommands,
		pendingBoardEditJob,
		pendingFlowIrCommit,
		backendContext.boardState,
		preflightPendingFlowIrCommit,
		dismissPendingFlowIrCommit,
		onApplyFlowIrCommit,
		recordExecutedBoardCommands,
		recordBoardApplyFailure,
		recordBoardApplySuccess,
		settleAuthoritativeBoardEditJob,
		settleGenerationReview,
	]);
	const handleExecuteCommands = useCallback(
		() => executePendingCommands(),
		[executePendingCommands],
	);

	const handleExecuteSingle = useCallback(
		async (index: number) => {
			// A compiled workflow commit is one atomic reviewed batch; never split its
			// lifecycle token across individual command buttons.
			if (pendingFlowIrCommit) {
				await handleExecuteCommands();
				return;
			}
			if (onExecuteCommands && pendingCommands[index]) {
				const command = pendingCommands[index];
				const lastAssistantMessage = [...messages]
					.reverse()
					.find(
						(message) =>
							message.role === "assistant" &&
							!isHostBoardApplyFeedbackMessage(message),
					);
				const nextExecutedCommands = [
					...(lastAssistantMessage?.executedCommands ?? []),
					command,
				];
				try {
					await onExecuteCommands([command]);
				} catch (error) {
					recordBoardApplyFailure(flowPilotCommandApplyDiagnostics(error));
					console.error("Failed to apply FlowPilot command:", error);
					return;
				}
				if (pendingCommands.length === 1) {
					settleGenerationReview("applied");
				}
				recordBoardApplySuccess();
				setMessages((prev) => {
					const newMessages = [...prev];
					for (let i = newMessages.length - 1; i >= 0; i--) {
						if (
							newMessages[i].role === "assistant" &&
							!isHostBoardApplyFeedbackMessage(newMessages[i])
						) {
							const existingCommands = newMessages[i].executedCommands || [];
							newMessages[i] = {
								...newMessages[i],
								executedCommands: [...existingCommands, command],
							};
							break;
						}
					}
					return newMessages;
				});
				if (currentMessageIdRef.current) {
					void updateMessage(currentMessageIdRef.current, {
						executedCommands: nextExecutedCommands,
					});
				}
				setPendingCommands((prev) => prev.filter((_, i) => i !== index));
			}
		},
		[
			handleExecuteCommands,
			messages,
			onExecuteCommands,
			pendingCommands,
			pendingFlowIrCommit,
			recordBoardApplyFailure,
			recordBoardApplySuccess,
			settleGenerationReview,
		],
	);

	const handleDismissCommands = useCallback(async () => {
		if (!(await dismissPendingFlowIrCommit())) return;
		settleGenerationReview("dismissed", pendingFlowIrCommit);
		if (flowscriptWorkspace) {
			setAppliedFlowScriptWorkspace(flowscriptWorkspace);
			setFlowscriptWorkspaceStatus("dismissed");
		}
		setPendingCommands([]);
		setDestructiveApplyRequest(null);
	}, [
		dismissPendingFlowIrCommit,
		flowscriptWorkspace,
		pendingFlowIrCommit,
		settleGenerationReview,
	]);

	const handleApproveFlowScriptDeletion = useCallback(async () => {
		if (!destructiveApplyRequest || !onApplyFlowScript) return;
		if (!(await preflightPendingFlowIrCommit())) return;

		setDestructiveApplyPending(true);
		try {
			const applyResult = await onApplyFlowScript(
				destructiveApplyRequest.flowscript,
				{ allowDeletions: true },
			);
			if (!applyResult) return;

			const diagnostics = applyResultDiagnostics(applyResult);
			if (
				applyResultCommandCount(applyResult) === 0 &&
				diagnostics.length > 0
			) {
				recordBoardApplyFailure(diagnostics);
				setFlowscriptWorkspaceStatus("validation_errors");
				setDestructiveApplyRequest(null);
				settleGenerationReview("error");
				return;
			}

			settleGenerationReview("applied");
			recordExecutedBoardCommands(applyResultBoardCommands(applyResult));
			setAppliedFlowScriptWorkspace(destructiveApplyRequest.flowscript);
			setFlowscriptWorkspaceStatus("applied");
			recordBoardApplySuccess();
			setPendingCommands([]);
			setDestructiveApplyRequest(null);
		} catch (error) {
			settleGenerationReview("error");
			recordBoardApplyFailure(flowPilotCommandApplyDiagnostics(error));
			console.error("Failed to apply destructive FlowScript edit:", error);
		} finally {
			setDestructiveApplyPending(false);
		}
	}, [
		destructiveApplyRequest,
		onApplyFlowScript,
		preflightPendingFlowIrCommit,
		recordExecutedBoardCommands,
		recordBoardApplyFailure,
		recordBoardApplySuccess,
		settleGenerationReview,
	]);

	// UI mode handlers
	const handleApplyComponents = useCallback(() => {
		if (pendingComponents.length > 0) {
			const nextAppliedComponents = [...pendingComponents];
			onApplyComponents?.(pendingComponents, pendingCanvasSettings);
			setMessages((prev) => {
				const newMessages = [...prev];
				for (let i = newMessages.length - 1; i >= 0; i--) {
					if (
						newMessages[i].role === "assistant" &&
						!isHostBoardApplyFeedbackMessage(newMessages[i])
					) {
						newMessages[i] = {
							...newMessages[i],
							appliedComponents: [...pendingComponents],
						};
						break;
					}
				}
				return newMessages;
			});
			if (currentMessageIdRef.current) {
				void updateMessage(currentMessageIdRef.current, {
					appliedComponents: nextAppliedComponents,
				});
			}
			setPendingComponents([]);
			setPendingCanvasSettings(undefined);
			setValidationWarnings([]);
		}
	}, [pendingComponents, pendingCanvasSettings, onApplyComponents]);

	const handleDismissComponents = useCallback(() => {
		setPendingComponents([]);
		setPendingCanvasSettings(undefined);
		setValidationWarnings([]);
	}, []);

	// Main submit handler
	const handleSubmit = useCallback(
		async (withScreenshot?: boolean) => {
			if (loading || (!input.trim() && attachedImages.length === 0)) return;

			let currentImages = [...attachedImages];
			const currentInput = input;
			const currentContextNodes = [...selectedNodeIds];
			const scope: CopilotScope =
				agentMode === "board"
					? "Board"
					: agentMode === "ui"
						? "Frontend"
						: "Both";

			if (scope === "Board" && !board) {
				setMessages((prev) => [
					...prev,
					{
						role: "assistant",
						content: "No board is currently loaded. Please load a board first.",
					},
				]);
				return;
			}
			if (pendingFlowIrCommit) {
				setValidationWarnings([
					"Apply or dismiss the pending compiled workflow before starting another board-generation turn.",
				]);
				return;
			}
			// A new request replaces any unreviewed raw FlowScript/direct-command preview. Retained
			// compiled workflow reviews are blocked above and must be resolved explicitly first.
			generationMetricsRunRef.current?.disposeReview("dismissed");
			generationMetricsRunRef.current = undefined;

			if (
				isAgentBackendProvider(normalizedProvider) &&
				(!copilotSDK.isRunning || !selectedModelId)
			) {
				setMessages((prev) => [
					...prev,
					{
						role: "assistant",
						content: copilotSDK.diagnostic
							? formatAgentBackendDiagnostic(copilotSDK.diagnostic)
							: "Agent backend is not ready yet. Connect the selected backend and choose a model before sending.",
					},
				]);
				return;
			}

			// Capture screenshot if requested and captureScreenshot is provided
			if (withScreenshot && captureScreenshot) {
				try {
					const screenshotDataUrl = await captureScreenshot();
					if (screenshotDataUrl) {
						// Parse the data URL
						const match = screenshotDataUrl.match(
							/^data:(image\/(?:png|jpe?g|webp|gif));base64,(.+)$/,
						);
						const mediaType = match
							? normalizedDataUrlImageType(match[1])
							: null;
						if (
							match &&
							mediaType &&
							base64ByteLength(match[2]) <= MAX_ATTACHED_IMAGE_BYTES &&
							currentImages.length < MAX_ATTACHED_IMAGES
						) {
							currentImages = [
								{
									data: match[2],
									mediaType,
									preview: screenshotDataUrl,
								},
								...currentImages,
							];
						}
					}
				} catch (error) {
					console.error("Failed to capture screenshot:", error);
				}
			}

			// Reset state first
			setInput("");
			setAttachedImages([]);
			setLoading(true);
			setLoadingPhase("initializing");
			setLoadingStartTime(Date.now());
			setPlanSteps([]);
			setProcessEvents([]);
			setInlineFlowScriptPreview(null);
			setUserScrolledUp(false);

			// In "both" mode, reset all pending states
			if (agentMode === "both") {
				setSuggestions([]);
				setPendingCommands([]);
				setPendingComponents([]);
				setValidationWarnings([]);
			} else if (agentMode === "board") {
				setSuggestions([]);
				setPendingCommands([]);
			} else {
				setPendingComponents([]);
				setValidationWarnings([]);
			}

			// Create or get conversation for persistence
			let conversationId = currentConversationId;
			if (!conversationId) {
				try {
					const newConversation = await createConversation(
						agentMode,
						board?.id,
						undefined,
					);
					conversationId = newConversation.id;
					setCurrentConversationId(conversationId);
					// Set initial title based on first message
					await updateConversation(conversationId, {
						title: currentInput.slice(0, 100) || "New conversation",
					});
				} catch (err) {
					console.error("Failed to create conversation:", err);
				}
			}

			// Save user message to DB
			if (conversationId) {
				try {
					await addMessage(conversationId, {
						role: "user",
						content: currentInput,
						images: currentImages.length > 0 ? currentImages : undefined,
						contextNodeIds:
							currentContextNodes.length > 0 ? currentContextNodes : undefined,
					});
					// Update conversation title if this is the first message
					if (messages.length === 0) {
						await updateConversation(conversationId, {
							title: currentInput.slice(0, 100),
						});
					}
				} catch (err) {
					console.error("Failed to save user message:", err);
				}
			}

			// Add user message and empty assistant message together
			setMessages((prev) => [
				...prev,
				{
					role: "user",
					content: currentInput,
					images: currentImages.length > 0 ? currentImages : undefined,
					contextNodeIds:
						currentContextNodes.length > 0 ? currentContextNodes : undefined,
				},
				{ role: "assistant", content: "" },
			]);

			// Store assistant message ID ref for updating later
			let assistantMessageId: string | undefined;
			if (conversationId) {
				try {
					const assistantMsg = await addMessage(conversationId, {
						role: "assistant",
						content: "",
					});
					assistantMessageId = assistantMsg.id;
					currentMessageIdRef.current = assistantMessageId;
				} catch (err) {
					console.error("Failed to create assistant message:", err);
				}
			}

			let phaseTimer: ReturnType<typeof setTimeout> | undefined;
			let draftingPreviewTimer: ReturnType<typeof setTimeout> | undefined;
			let pendingDraftingPreview: InlineFlowScriptPreviewValue | undefined;
			let hasAuthoritativeFlowScriptWorkspace = false;
			let generationMetricsRun: FlowPilotGenerationMetricsRun | undefined;
			let ownedCopilotRequestId: string | undefined;
			try {
				let currentMessageContent = "";
				let lastUpdateTime = 0;
				const UPDATE_INTERVAL = 100;
				const streamParser = createCopilotStreamParser();
				let currentPlanSteps: UnifiedPlanStep[] = [];
				let latestFlowScriptWorkspace = flowscriptWorkspace;
				let latestAuthoritativeFlowScriptWorkspace = flowscriptWorkspace;
				let generatedFlowScriptWorkspaceStatus: string | undefined;
				let workspaceCandidates: FlowScriptWorkspaceCandidate[] = [];
				let currentProcessEvents: FlowPilotProcessEvent[] = [];

				phaseTimer = setTimeout(() => setLoadingPhase("analyzing"), 300);

				const flushMessageContent = () => {
					setMessages((prev) => {
						const newMessages = [...prev];
						const lastMessage = newMessages[newMessages.length - 1];
						if (lastMessage && lastMessage.role === "assistant") {
							lastMessage.content = currentMessageContent;
						}
						return newMessages;
					});
				};

				const syncProcessEvents = (events: FlowPilotProcessEvent[]) => {
					currentProcessEvents = events.slice(-60);
					setProcessEvents(currentProcessEvents);
					setMessages((prev) => {
						const newMessages = [...prev];
						const lastMessage = newMessages[newMessages.length - 1];
						if (lastMessage && lastMessage.role === "assistant") {
							lastMessage.processEvents = currentProcessEvents;
						}
						return newMessages;
					});
				};

				const appendProcessEvent = (
					event: Omit<FlowPilotProcessEvent, "createdAt"> & {
						createdAt?: number;
					},
				) => {
					syncProcessEvents([
						...currentProcessEvents,
						{
							...event,
							createdAt: event.createdAt ?? Date.now(),
						},
					]);
				};

				const updateProcessEvent = (
					id: string,
					patch:
						| Partial<FlowPilotProcessEvent>
						| ((
								event: FlowPilotProcessEvent,
						  ) => Partial<FlowPilotProcessEvent>),
				) => {
					let found = false;
					const nextEvents = currentProcessEvents.map((event) => {
						if (event.id !== id) return event;
						found = true;
						const resolvedPatch =
							typeof patch === "function" ? patch(event) : patch;
						return {
							...event,
							...resolvedPatch,
							updatedAt: Date.now(),
						};
					});
					if (found) {
						syncProcessEvents(nextEvents);
					}
					return found;
				};

				const publishDraftingPreview = () => {
					draftingPreviewTimer = undefined;
					const preview = pendingDraftingPreview;
					pendingDraftingPreview = undefined;
					if (!preview) return;
					setFlowscriptWorkspace(preview.source);
					setFlowscriptWorkspaceStatus("drafting");
					setInlineFlowScriptPreview(preview);
					setShowWorkspace(true);
				};

				const scheduleDraftingPreview = (
					preview: InlineFlowScriptPreviewValue,
				) => {
					pendingDraftingPreview = preview;
					if (draftingPreviewTimer) return;
					draftingPreviewTimer = setTimeout(
						publishDraftingPreview,
						FLOWSCRIPT_DRAFT_PREVIEW_INTERVAL_MS,
					);
				};

				const discardPendingDraftingPreview = () => {
					if (draftingPreviewTimer) clearTimeout(draftingPreviewTimer);
					draftingPreviewTimer = undefined;
					pendingDraftingPreview = undefined;
				};

				const applyFlowScriptWorkspace = (
					workspace: string,
					status?: string,
				) => {
					const source = workspace;
					if (!source.trim()) return;
					const preview = { source, status };
					const previousWorkspace = latestAuthoritativeFlowScriptWorkspace;
					latestFlowScriptWorkspace = source;
					generatedFlowScriptWorkspaceStatus = status ?? "submitted";
					if (isDraftingFlowScriptWorkspace(status)) {
						// Draft snapshots are display-only. Coalesce renderer updates and never let an
						// incomplete JSON-argument stream enter recovery, history, or the apply path.
						scheduleDraftingPreview(preview);
						return;
					}

					discardPendingDraftingPreview();
					hasAuthoritativeFlowScriptWorkspace = true;
					latestAuthoritativeFlowScriptWorkspace = source;
					workspaceCandidates = rememberFlowScriptWorkspaceCandidate(
						workspaceCandidates,
						{ source, status },
					);
					setFlowscriptWorkspace(source);
					setFlowscriptWorkspaceStatus(status ?? "submitted");
					setInlineFlowScriptPreview(preview);
					setShowWorkspace(true);
					if (previousWorkspace !== source) {
						appendProcessEvent({
							id: `workspace-${Date.now()}`,
							kind: "workspace",
							status: "done",
							title: previousWorkspace
								? "Updated FlowScript workspace"
								: "Created FlowScript workspace",
							summary: previousWorkspace
								? `${formatLineCount(previousWorkspace)} -> ${formatLineCount(source)}`
								: formatLineCount(source),
							workspaceBefore: previousWorkspace,
							workspaceAfter: source,
						});
					}
					setMessages((prev) => {
						const newMessages = [...prev];
						const lastMessage = newMessages[newMessages.length - 1];
						if (lastMessage && lastMessage.role === "assistant") {
							lastMessage.flowscriptWorkspace = source;
						}
						return newMessages;
					});
					if (assistantMessageId) {
						void updateMessage(assistantMessageId, {
							flowscriptWorkspace: source,
						});
					}
				};

				const processStreamToken = (rawToken: string) => {
					let token = rawToken;
					// Parse scope decision events (skip them - they're internal)
					const scopeDecisionMatch = token.match(
						/<scope_decision>([\s\S]*?)<\/scope_decision>/,
					);
					if (scopeDecisionMatch) {
						return;
					}

					// Parse FlowScript workspace updates
					const workspaceFrames = extractFlowScriptWorkspaceCandidates(token);
					if (workspaceFrames.candidates.length > 0) {
						for (const workspaceEvent of workspaceFrames.candidates) {
							applyFlowScriptWorkspace(
								workspaceEvent.source,
								workspaceEvent.status,
							);
						}
						token = workspaceFrames.remainder;
						if (!token.trim()) return;
					}

					// Parse tool start events (Copilot SDK)
					const toolStartMatch = token.match(
						/<tool_start>([\s\S]*?)<\/tool_start>/,
					);
					if (toolStartMatch) {
						try {
							const eventData = parseStreamJson(toolStartMatch[1]);
							const toolName =
								typeof eventData?.tool === "string" ? eventData.tool : "tool";
							const toolCallId =
								typeof eventData?.tool_call_id === "string"
									? eventData.tool_call_id
									: `tool-${Date.now()}`;
							setCurrentToolCall(toolName);
							appendProcessEvent({
								id: toolCallId,
								kind: "tool",
								status: "running",
								title: getProcessToolLabel(toolName),
								toolName,
								summary: stringifyPreview(eventData?.summary),
								details: stringifyPreview(eventData?.arguments_preview),
							});
							// Update loading phase based on tool name
							if (
								toolName.includes("search") ||
								toolName.includes("catalog") ||
								toolName === "get_declarations" ||
								toolName === "internet_search" ||
								toolName === "open_url" ||
								toolName === "archive_lookup"
							) {
								setLoadingPhase("searching");
							} else if (
								toolName === "get_node_details" ||
								toolName === "list_board_nodes" ||
								toolName === "get_component_schema" ||
								toolName === "database_tool" ||
								toolName === "storage_tool" ||
								toolName === "ask_user"
							) {
								setLoadingPhase("reasoning");
							} else if (
								toolName === "emit_commands" ||
								toolName === "emit_ui" ||
								toolName === "write_flowscript" ||
								toolName === "patch_flowscript" ||
								toolName === "check_flowscript" ||
								toolName === "commit_flowscript" ||
								toolName === "edit_flowscript" ||
								toolName === "execute_event" ||
								toolName === "execute_node"
							) {
								setLoadingPhase("generating");
							} else if (
								toolName === "get_unconfigured_nodes" ||
								toolName === "query_execution_logs"
							) {
								setLoadingPhase("searching");
							}
						} catch {
							// Invalid JSON
						}
						return;
					}

					// Parse tool progress events (Copilot SDK)
					const toolProgressMatch = token.match(
						/<tool_progress>([\s\S]*?)<\/tool_progress>/,
					);
					if (toolProgressMatch) {
						try {
							const eventData = parseStreamJson(toolProgressMatch[1]);
							const toolCallId =
								typeof eventData?.tool_call_id === "string"
									? eventData.tool_call_id
									: undefined;
							const message = stringifyPreview(eventData?.message);
							if (toolCallId && message) {
								const updated = updateProcessEvent(toolCallId, (event) => ({
									summary: message,
									details: appendBoundedStreamDetail(event.details, message),
								}));
								if (!updated) {
									appendProcessEvent({
										id: toolCallId,
										kind: "progress",
										status: "running",
										title: "Tool progress",
										summary: message,
									});
								}
							}
						} catch {
							// Invalid JSON
						}
						return;
					}

					// Parse tool end events (Copilot SDK)
					const toolEndMatch = token.match(/<tool_end>([\s\S]*?)<\/tool_end>/);
					if (toolEndMatch) {
						try {
							const eventData = parseStreamJson(toolEndMatch[1]);
							const toolCallId =
								typeof eventData?.tool_call_id === "string"
									? eventData.tool_call_id
									: undefined;
							const toolName =
								typeof eventData?.tool === "string"
									? eventData.tool
									: undefined;
							const status = eventData?.status === "error" ? "error" : "done";
							const resultSummary =
								stringifyPreview(eventData?.result_summary) ??
								stringifyPreview(eventData?.error);
							if (toolCallId) {
								const updated = updateProcessEvent(toolCallId, {
									status,
									title: getProcessToolLabel(toolName),
									toolName,
									summary: resultSummary,
									resultPreview: stringifyPreview(eventData?.result_preview),
								});
								if (!updated) {
									appendProcessEvent({
										id: toolCallId,
										kind: "tool",
										status,
										title: getProcessToolLabel(toolName),
										toolName,
										summary: resultSummary,
										resultPreview: stringifyPreview(eventData?.result_preview),
									});
								}
							}
							setTimeout(() => setCurrentToolCall(null), 500);
						} catch {
							// Invalid JSON
						}
						return;
					}

					// Parse plan step events
					const planStepMatch = token.match(
						/<plan_step>([\s\S]*?)<\/plan_step>/,
					);
					if (planStepMatch) {
						try {
							const eventData = JSON.parse(planStepMatch[1]);
							if (eventData.PlanStep) {
								const step = eventData.PlanStep;
								// Update loading phase based on tool
								if (
									step.tool_name === "think" ||
									step.tool_name === "analyze"
								) {
									setLoadingPhase("reasoning");
								} else if (
									step.tool_name?.includes("search") ||
									step.tool_name?.includes("catalog") ||
									step.tool_name?.includes("schema") ||
									step.tool_name?.includes("style")
								) {
									setLoadingPhase("searching");
								} else if (
									step.tool_name === "emit_commands" ||
									step.tool_name === "emit_surface" ||
									step.tool_name === "write_flowscript" ||
									step.tool_name === "patch_flowscript" ||
									step.tool_name === "check_flowscript" ||
									step.tool_name === "commit_flowscript" ||
									step.tool_name === "edit_flowscript" ||
									step.tool_name === "modify_component"
								) {
									setLoadingPhase("generating");
								}

								const existingIndex = currentPlanSteps.findIndex(
									(s) => s.id === step.id,
								);
								currentPlanSteps =
									existingIndex >= 0
										? currentPlanSteps.map((existing, index) =>
												index === existingIndex ? step : existing,
											)
										: [...currentPlanSteps, step];
								setPlanSteps(currentPlanSteps);

								const processEventId = `plan-${step.id}`;
								const processStatus = processStatusFromPlanStepStatus(
									step.status,
								);
								const processPatch: Partial<FlowPilotProcessEvent> = {
									kind: step.tool_name ? "tool" : "progress",
									status: processStatus,
									title: getProcessToolLabel(step.tool_name),
									toolName: step.tool_name,
									summary: step.description,
								};
								const updated = updateProcessEvent(
									processEventId,
									processPatch,
								);
								if (!updated) {
									appendProcessEvent({
										id: processEventId,
										kind: processPatch.kind ?? "progress",
										status: processStatus,
										title: processPatch.title ?? "Working",
										toolName: step.tool_name,
										summary: step.description,
									});
								}
							}
						} catch {
							// Invalid JSON
						}
						return;
					}

					// Parse command blocks from Copilot SDK emit_commands tool
					const commandsMatch = token.match(/<commands>([\s\S]*?)<\/commands>/);
					if (commandsMatch) {
						try {
							const commands = JSON.parse(commandsMatch[1]);
							if (Array.isArray(commands) && commands.length > 0) {
								const flowScriptOwnsApply = flowScriptWorkspaceOwnsApply(
									latestFlowScriptWorkspace,
									generatedFlowScriptWorkspaceStatus,
								);
								if (!flowScriptOwnsApply) {
									setPendingCommands((prev) => [...prev, ...commands]);
								}
								appendProcessEvent({
									id: `commands-${Date.now()}`,
									kind: "commands",
									status: "done",
									title: flowScriptOwnsApply
										? "Derived FlowScript changes"
										: "Queued board changes",
									summary: `${commands.length} change${
										commands.length === 1 ? "" : "s"
									} ${
										flowScriptOwnsApply
											? "derived from FlowScript"
											: "ready for review"
									}`,
									commands,
								});
							}
						} catch {
							// Invalid JSON in commands
						}
						// Remove the commands tag from the token but keep any surrounding text
						const cleanedToken = token.replace(
							/<commands>[\s\S]*?<\/commands>/g,
							"",
						);
						if (!cleanedToken.trim()) return;
						// Continue with the cleaned token
						currentMessageContent += cleanedToken;
						flushMessageContent();
						return;
					}

					// Parse component blocks from Copilot SDK emit_ui tool
					const componentsMatch = token.match(
						/<components>([\s\S]*?)<\/components>/,
					);
					if (componentsMatch) {
						try {
							const components = JSON.parse(componentsMatch[1]);
							if (Array.isArray(components) && components.length > 0) {
								const { components: validatedBatch, warnings } =
									validateComponents(components);
								if (validatedBatch.length > 0) {
									setPendingComponents((prev) => [...prev, ...validatedBatch]);
									appendProcessEvent({
										id: `components-${Date.now()}`,
										kind: "components",
										status: "done",
										title: "Generated UI components",
										summary: `${validatedBatch.length} component${validatedBatch.length === 1 ? "" : "s"} ready for review`,
										componentCount: validatedBatch.length,
									});
								}
								if (warnings.length > 0) {
									setValidationWarnings((prev) => [...prev, ...warnings]);
								}
							}
						} catch {
							// Invalid JSON in components
						}
						// Remove the components tag from the token but keep any surrounding text
						const cleanedToken = token.replace(
							/<components>[\s\S]*?<\/components>/g,
							"",
						);
						if (!cleanedToken.trim()) return;
						currentMessageContent += cleanedToken;
						flushMessageContent();
						return;
					}

					// Parse canvas_settings blocks from Copilot SDK emit_ui tool
					const canvasSettingsMatch = token.match(
						/<canvas_settings>([\s\S]*?)<\/canvas_settings>/,
					);
					if (canvasSettingsMatch) {
						try {
							const settings = JSON.parse(canvasSettingsMatch[1]);
							setPendingCanvasSettings(validateCanvasSettings(settings));
						} catch {
							// Invalid JSON in canvas settings
						}
						// Remove the tag and continue
						const cleanedToken = token.replace(
							/<canvas_settings>[\s\S]*?<\/canvas_settings>/g,
							"",
						);
						if (!cleanedToken.trim()) return;
						currentMessageContent += cleanedToken;
						flushMessageContent();
						return;
					}

					// First token? Set loading phase immediately
					if (currentMessageContent.length === 0 && token.trim()) {
						setLoadingPhase("generating");
						// Flush immediately on first content to avoid losing it
						currentMessageContent += token;
						flushMessageContent();
						return;
					}

					currentMessageContent += token;

					// Throttle UI updates
					const now = Date.now();
					if (now - lastUpdateTime >= UPDATE_INTERVAL) {
						lastUpdateTime = now;
						flushMessageContent();
					}
				};

				const processStreamEvent = (event: CopilotStreamEvent) => {
					if (event.type === "text") {
						if (event.text) processStreamToken(event.text);
						return;
					}
					if (event.type === "usage_stat") return;
					processStreamToken(
						`<${event.type}>${event.raw ?? ""}</${event.type}>`,
					);
				};

				const onToken = (rawToken: string) => {
					generationMetricsRun?.push(rawToken);
					for (const event of streamParser.push(rawToken)) {
						processStreamEvent(event);
					}
				};

				// Scope and run data already travel in trusted request context and system guidance. Keep
				// the user message immutable so host wrappers cannot influence routing or acceptance.
				const userMsg = currentInput;

				const chatHistory: UnifiedChatMessage[] = buildBudgetedHistory({
					agentMode,
					messages,
					selectedNodeIds,
					selectedComponentIds,
					boardId: board?.id,
					boardName: board?.name,
					currentComponentsCount: currentComponents.length,
					runContext,
				});

				const requestImages = currentImages.map((img) => ({
					data: img.data,
					media_type: img.mediaType,
				}));

				const backendRunContext = runContext
					? {
							run_id: runContext.run_id,
							app_id: runContext.app_id,
							board_id: runContext.board_id,
						}
					: undefined;

				const effectiveModelId = flowPilotModelIdForProvider(
					normalizedProvider,
					selectedModelId,
				);

				const nativeRequestId = `${DIRECT_FLOWPILOT_BOARD_EDIT_REQUEST_PREFIX}${createId()}`;
				ownedCopilotRequestId = nativeRequestId;
				generationMetricsRun = new FlowPilotGenerationMetricsRun(
					nativeRequestId,
				);
				generationMetricsRunRef.current = generationMetricsRun;
				// Own the generation before frontend context collection begins. Stop, unmount, or a
				// board change can now revoke this id while the bounded collector is awaiting IO.
				activeCopilotRequestIdRef.current = nativeRequestId;
				const boardContextManifest =
					(agentMode === "board" || agentMode === "both") &&
					activeAppId &&
					board?.id
						? await buildFlowPilotBoardContextAugmentation(
								executeRuntimeTool,
								activeAppId,
								board.id,
								`${board.id}:${Object.keys(board.nodes ?? {}).length}:${Object.keys(board.layers ?? {}).length}`,
							)
						: undefined;
				if (activeCopilotRequestIdRef.current !== nativeRequestId) {
					throw new Error(
						"FlowPilot request was cancelled before model generation because its board or session changed.",
					);
				}
				let response: Awaited<
					ReturnType<typeof backendContext.boardState.copilot_chat>
				>;
				response = await backendContext.boardState.copilot_chat(
					scope,
					board ?? null,
					catalogNodes,
					selectedNodeIds,
					currentComponents,
					selectedComponentIds,
					userMsg,
					chatHistory,
					requestImages,
					onToken,
					effectiveModelId,
					selectedReasoningEffort || undefined,
					undefined,
					backendRunContext,
					undefined, // actionContext - can be added later
					undefined, // nested
					undefined, // Auto intent: native host resolves question vs authoring once for every backend.
					activeAppId
						? {
								appId: activeAppId,
								boardId: board?.id,
								conversationId: flowPilotPanelConversationId(
									board?.id ?? activeAppId,
								),
								sourceUserPrompt: currentInput,
								boardContextManifest,
							}
						: undefined,
					nativeRequestId,
					currentInput,
					activeAppId,
				);
				const responseBelongsToActiveRequest =
					activeCopilotRequestIdRef.current === nativeRequestId;
				const staleDismissal =
					await releaseReturnedFlowIrCommitBeforeStaleResponse(
						responseBelongsToActiveRequest,
						response.flow_ir_commit,
						dismissFlowIrCommitWithRetry,
					);
				if (!responseBelongsToActiveRequest) {
					if (
						response.flow_ir_commit &&
						staleDismissal?.status !== "dismissed" &&
						staleDismissal?.code !== "IR_COMMIT_TOKEN_INVALID"
					) {
						pendingFlowIrCommitRef.current = response.flow_ir_commit;
						setPendingFlowIrCommit(response.flow_ir_commit);
						setValidationWarnings((warnings) => [
							...warnings,
							staleDismissal?.message ||
								"The stale compiled workflow review could not be released; dismiss it before generating another workflow.",
						]);
					}
					generationMetricsRun.finish(
						"cancelled",
						Boolean(response.flow_ir_commit),
						currentBoardNodeCountRef.current,
					);
					if (response.flow_ir_commit) {
						generationMetricsRun.disposeReview(
							"stale",
							response.flow_ir_commit,
							currentBoardNodeCountRef.current,
						);
					}
					if (generationMetricsRunRef.current === generationMetricsRun) {
						generationMetricsRunRef.current = undefined;
					}
					throw new Error(
						"FlowPilot request was cancelled because its board or session changed.",
					);
				}
				if (
					response.flow_ir_commit &&
					response.flow_ir_commit.board_id !== currentBoardIdRef.current
				) {
					const mismatchedToken = response.flow_ir_commit;
					const dismissal = await dismissFlowIrCommitWithRetry(mismatchedToken);
					if (
						dismissal.status !== "dismissed" &&
						dismissal.code !== "IR_COMMIT_TOKEN_INVALID"
					) {
						pendingFlowIrCommitRef.current = mismatchedToken;
						setPendingFlowIrCommit(mismatchedToken);
					}
					generationMetricsRun.finish(
						"cancelled",
						true,
						currentBoardNodeCountRef.current,
					);
					generationMetricsRun.disposeReview(
						"stale",
						mismatchedToken,
						currentBoardNodeCountRef.current,
					);
					if (generationMetricsRunRef.current === generationMetricsRun) {
						generationMetricsRunRef.current = undefined;
					}
					throw new Error(
						"FlowPilot finished against a board that is no longer open; its compiled workflow review was dismissed.",
					);
				}

				for (const event of streamParser.flush()) processStreamEvent(event);
				flushMessageContent();

				const finalAssistantContent =
					currentMessageContent || response.message || "";
				if (
					response.flow_ir_commit &&
					activeAppId &&
					backendContext.boardState.createBoardEditJob
				) {
					let createdJob: BoardEditJob | undefined;
					try {
						createdJob = await backendContext.boardState.createBoardEditJob(
							activeAppId,
							nativeRequestId,
							response.flow_ir_commit,
						);
						if (
							activeCopilotRequestIdRef.current !== nativeRequestId ||
							currentBoardIdRef.current !== createdJob.boardId
						) {
							throw new Error(
								"FlowPilot detached after creating a durable board-edit review. Reopen its board to continue the review.",
							);
						}
						pendingBoardEditJobRef.current = createdJob;
						setPendingBoardEditJob(createdJob);
					} catch (error) {
						if (
							activeCopilotRequestIdRef.current !== nativeRequestId ||
							currentBoardIdRef.current !== response.flow_ir_commit.board_id
						) {
							// A successfully created native job owns the exact token and will rehydrate on
							// its board. Only release the claim if native job creation itself failed.
							if (!createdJob) {
								await dismissFlowIrCommitWithRetry(response.flow_ir_commit);
							}
							throw error;
						}
						// Compatibility fallback: the exact commit token still uses the existing native
						// Apply/Dismiss path on hosts that predate resumable review jobs.
						console.warn(
							"Failed to create resumable FlowPilot board-edit review:",
							error,
						);
						pendingBoardEditJobRef.current = undefined;
						setPendingBoardEditJob(undefined);
					}
				} else if (!response.flow_ir_commit) {
					pendingBoardEditJobRef.current = undefined;
					setPendingBoardEditJob(undefined);
				}
				pendingFlowIrCommitRef.current = response.flow_ir_commit;
				setPendingFlowIrCommit(response.flow_ir_commit);
				if (response.flowscript_workspace) {
					const finalWorkspace = resolveFinalFlowScriptWorkspaceCandidate(
						workspaceCandidates,
						response.flowscript_workspace,
						(response.commands?.length ?? 0) > 0,
					);
					if (finalWorkspace) {
						applyFlowScriptWorkspace(
							finalWorkspace.source,
							finalWorkspace.status,
						);
					}
				}

				// Save final assistant message to DB. Persist the process timeline and completed
				// plan steps too, so a workflow-edit turn (whose visible output is the timeline, not
				// prose) still renders when reloaded from history. Large string fields inside these
				// are offloaded to disk by the Dexie blob-offload middleware.
				const completedPlanSteps = currentPlanSteps.filter(
					(s) => s.status === "Completed",
				);
				if (
					assistantMessageId &&
					(finalAssistantContent ||
						currentProcessEvents.length > 0 ||
						hasAuthoritativeFlowScriptWorkspace)
				) {
					try {
						await updateMessage(assistantMessageId, {
							content: finalAssistantContent,
							flowscriptWorkspace: hasAuthoritativeFlowScriptWorkspace
								? latestFlowScriptWorkspace
								: undefined,
							processEvents:
								currentProcessEvents.length > 0
									? currentProcessEvents
									: undefined,
							planSteps:
								completedPlanSteps.length > 0 ? completedPlanSteps : undefined,
						});
					} catch (err) {
						console.error("Failed to update assistant message:", err);
					}
				}

				setMessages((prev) => {
					const newMessages = [...prev];
					const lastMessage = newMessages[newMessages.length - 1];
					if (lastMessage && lastMessage.role === "assistant") {
						lastMessage.planSteps = currentPlanSteps.filter(
							(s) => s.status === "Completed",
						);
						if (!lastMessage.content.trim()) {
							lastMessage.content = finalAssistantContent;
						}
						if (hasAuthoritativeFlowScriptWorkspace) {
							lastMessage.flowscriptWorkspace = latestFlowScriptWorkspace;
						}
						if (currentProcessEvents.length > 0) {
							lastMessage.processEvents = currentProcessEvents;
						}
					}
					return newMessages;
				});

				// Handle board commands
				if (
					response.commands.length > 0 &&
					(response.flow_ir_commit ||
						!flowScriptWorkspaceOwnsApply(
							latestFlowScriptWorkspace,
							generatedFlowScriptWorkspaceStatus,
						))
				) {
					setPendingCommands(response.commands);
				}

				// Handle suggestions
				if (response.suggestions?.length > 0) {
					setSuggestions(
						response.suggestions.map((s) => ({
							node_type: s.label,
							reason: s.prompt,
							connection_description: "",
							connections: [],
						})),
					);
				}

				const validatedCanvasSettings = validateCanvasSettings(
					response.canvas_settings,
				);
				if (validatedCanvasSettings) {
					setPendingCanvasSettings(validatedCanvasSettings);
				}

				// Handle generated components — validate before showing
				if (response.components.length > 0) {
					const { components: validatedFinal, warnings: finalWarnings } =
						validateComponents(response.components);
					if (validatedFinal.length > 0) {
						setPendingComponents(validatedFinal);
						onComponentsGenerated?.(validatedFinal);
					}
					if (finalWarnings.length > 0) {
						setValidationWarnings((prev) => [...prev, ...finalWarnings]);
					}
				}

				const awaitingWorkflowReview =
					(agentMode === "board" || agentMode === "both") &&
					(Boolean(response.flow_ir_commit) ||
						response.commands.length > 0 ||
						generatedFlowScriptWorkspaceStatus === "queued");
				generationMetricsRun.finish(
					generatedFlowScriptWorkspaceStatus === "validation_errors"
						? "partial"
						: "ok",
					awaitingWorkflowReview,
					currentBoardNodeCountRef.current,
				);
				if (
					!awaitingWorkflowReview &&
					generationMetricsRunRef.current === generationMetricsRun
				) {
					generationMetricsRunRef.current = undefined;
				}

				setLoadingPhase("finalizing");
			} catch (error) {
				if (draftingPreviewTimer) clearTimeout(draftingPreviewTimer);
				draftingPreviewTimer = undefined;
				if (pendingDraftingPreview) {
					const interruptedPreview = {
						...pendingDraftingPreview,
						status: "interrupted",
					};
					pendingDraftingPreview = undefined;
					setFlowscriptWorkspace(interruptedPreview.source);
					setFlowscriptWorkspaceStatus("interrupted");
					setInlineFlowScriptPreview(interruptedPreview);
					setShowWorkspace(true);
				} else {
					setFlowscriptWorkspaceStatus((status) =>
						status === "drafting" ? "interrupted" : status,
					);
					setInlineFlowScriptPreview((preview) =>
						preview?.status === "drafting"
							? { ...preview, status: "interrupted" }
							: preview,
					);
				}
				const generationError =
					error instanceof Error ? error.message.toLowerCase() : "";
				generationMetricsRun?.finish(
					generationError.includes("timeout") ||
						generationError.includes("timed out")
						? "timeout"
						: generationError.includes("cancel")
							? "cancelled"
							: "error",
					false,
					currentBoardNodeCountRef.current,
				);
				if (generationMetricsRunRef.current === generationMetricsRun) {
					generationMetricsRunRef.current = undefined;
				}
				console.error("FlowPilot error:", error);
				// Streamed command frames are previews until the final response transfers any
				// retained compiled workflow review token. A failed/cancelled transport releases
				// that native claim, so keeping those commands applyable would bypass stale-board
				// preflight.
				if (agentMode === "board" || agentMode === "both") {
					const transferredToken = pendingFlowIrCommitRef.current;
					const transferredJob = pendingBoardEditJobRef.current;
					if (
						transferredToken &&
						transferredJob?.token.claim_id !== transferredToken.claim_id
					) {
						const dismissal = await resolveFlowIrCommit(
							transferredToken,
							"dismissed",
						);
						if (dismissal.status === "dismissed") {
							pendingFlowIrCommitRef.current = undefined;
							setPendingFlowIrCommit(undefined);
						} else {
							setPendingFlowIrCommit(transferredToken);
							setValidationWarnings((warnings) => [
								...warnings,
								dismissal.message ||
									"The interrupted compiled workflow review could not be released. Dismiss it before generating another workflow.",
							]);
						}
					}
					setPendingCommands([]);
					if (!transferredJob) {
						setFlowscriptWorkspaceStatus((status) =>
							status === "queued" ? "stale" : status,
						);
					}
				}
				setMessages((prev) => {
					const newMessages = [...prev];
					const lastMessage = newMessages[newMessages.length - 1];
					if (lastMessage?.role === "assistant") {
						let errorMessage =
							error instanceof Error
								? error.message
								: typeof error === "string"
									? error
									: String(error ?? "Unknown error");

						if (isAgentBackendProvider(normalizedProvider)) {
							const backendDiagnostic = classifyAgentBackendError(
								normalizedProvider,
								errorMessage,
							);
							if (backendDiagnostic) {
								if (shouldPersistAgentBackendDiagnostic(backendDiagnostic)) {
									copilotBackendConnectionCoordinator.reportFailure(
										normalizedProvider,
										error,
									);
								}
								errorMessage = formatAgentBackendDiagnostic(backendDiagnostic);
							}
						}

						lastMessage.content = `Error: ${errorMessage}`;
						if (assistantMessageId) {
							void updateMessage(assistantMessageId, {
								content: lastMessage.content,
							});
						}
					}
					return newMessages;
				});
			} finally {
				if (
					ownedCopilotRequestId &&
					activeCopilotRequestIdRef.current === ownedCopilotRequestId
				) {
					activeCopilotRequestIdRef.current = undefined;
				}
				if (phaseTimer) clearTimeout(phaseTimer);
				if (draftingPreviewTimer) clearTimeout(draftingPreviewTimer);
				if (pendingDraftingPreview && !hasAuthoritativeFlowScriptWorkspace) {
					const interruptedPreview = {
						...pendingDraftingPreview,
						status: "interrupted",
					};
					setFlowscriptWorkspace(interruptedPreview.source);
					setFlowscriptWorkspaceStatus("interrupted");
					setInlineFlowScriptPreview(interruptedPreview);
					setShowWorkspace(true);
				}
				setLoading(false);
				setLoadingPhase("idle");
				setLoadingStartTime(null);
				setCurrentToolCall(null);
			}
		},
		[
			input,
			attachedImages,
			agentMode,
			messages,
			board,
			selectedNodeIds,
			selectedModelId,
			selectedReasoningEffort,
			runContext,
			currentComponents,
			selectedComponentIds,
			activeAppId,
			onComponentsGenerated,
			backendContext.boardState,
			captureScreenshot,
			provider,
			normalizedProvider,
			copilotSDK.isRunning,
			copilotSDK.diagnostic,
			currentConversationId,
			flowscriptWorkspace,
			flowscriptWorkspaceStatus,
			loading,
			pendingFlowIrCommit,
			resolveFlowIrCommit,
			dismissFlowIrCommitWithRetry,
			activeAppId,
		],
	);

	// Keep ref updated for keydown handler
	useEffect(() => {
		handleSubmitRef.current = handleSubmit;
	}, [handleSubmit]);

	// Handle key down
	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent<HTMLTextAreaElement>) => {
			if (e.key === "Enter" && !e.shiftKey) {
				e.preventDefault();
				handleSubmitRef.current?.();
			}
		},
		[],
	);

	// Handle initial prompt
	useEffect(() => {
		if (
			initialPrompt &&
			!initialPromptHandledRef.current &&
			selectedModelId &&
			(agentMode === "ui" || board)
		) {
			initialPromptHandledRef.current = true;
			setInput(initialPrompt);
			setTimeout(() => {
				handleSubmitRef.current?.();
			}, 100);
		}
	}, [initialPrompt, selectedModelId, agentMode, board]);

	// Get placeholder text based on mode
	const placeholderText = useMemo(() => {
		if (agentMode === "both") {
			if (runContext) return "Ask about logs or describe what to build...";
			const hasSelection =
				selectedNodeIds.length > 0 || selectedComponentIds.length > 0;
			if (hasSelection) return "Describe changes to selected items...";
			return "Describe a workflow, UI, or both together...";
		}
		if (agentMode === "board") {
			if (runContext) return "Ask about the logs...";
			if (selectedNodeIds.length > 0)
				return "Describe changes to selected nodes...";
			return "Ask anything about your flow...";
		}
		if (selectedComponentIds.length > 0) {
			return "Describe changes to selected components...";
		}
		return "Describe the UI you want to create...";
	}, [
		agentMode,
		runContext,
		selectedNodeIds.length,
		selectedComponentIds.length,
	]);

	// Get context indicator based on mode
	const contextIndicator = useMemo(() => {
		// In "both" mode, show combined context
		if (agentMode === "both") {
			const hasNodes = selectedNodeIds.length > 0;
			const hasComponents = selectedComponentIds.length > 0;
			if (!hasNodes && !hasComponents) return null;

			return (
				<div className="flex items-center gap-2 mb-2 flex-wrap">
					{hasNodes && (
						<ContextNodes
							nodeIds={selectedNodeIds}
							board={board ?? undefined}
							onSelectNodes={onSelectNodes}
							onFocusNode={onFocusNode}
							compact
						/>
					)}
					{hasComponents && (
						<div className="flex items-center gap-1.5 text-xs text-muted-foreground">
							<LayoutGridIcon className="w-3.5 h-3.5" />
							<span>
								{selectedComponentIds.length} component
								{selectedComponentIds.length !== 1 ? "s" : ""}
							</span>
						</div>
					)}
				</div>
			);
		}
		if (agentMode === "board" && selectedNodeIds.length > 0) {
			return (
				<ContextNodes
					nodeIds={selectedNodeIds}
					board={board ?? undefined}
					onSelectNodes={onSelectNodes}
					onFocusNode={onFocusNode}
					compact
				/>
			);
		}
		if (agentMode === "ui" && selectedComponentIds.length > 0) {
			return (
				<div className="flex items-center gap-1.5 mb-2 text-xs text-muted-foreground">
					<LayoutGridIcon className="w-3.5 h-3.5" />
					<span>
						{selectedComponentIds.length} component
						{selectedComponentIds.length !== 1 ? "s" : ""} selected
					</span>
				</div>
			);
		}
		return null;
	}, [
		agentMode,
		selectedNodeIds,
		selectedComponentIds,
		board,
		onSelectNodes,
		onFocusNode,
	]);

	const hasFlowScriptWorkspace =
		Boolean(flowscriptWorkspace) &&
		(agentMode === "board" || agentMode === "both");
	const hasUnappliedFlowScriptWorkspace =
		hasFlowScriptWorkspace &&
		!pendingFlowIrCommit &&
		Boolean(onApplyFlowScript) &&
		isFlowScriptWorkspaceApplicable({
			source: flowscriptWorkspace,
			status: flowscriptWorkspaceStatus,
		}) &&
		flowscriptWorkspace !== appliedFlowScriptWorkspace;
	const showFlowScriptWorkspace = hasFlowScriptWorkspace && showWorkspace;
	const visiblePendingCommands = hasUnappliedFlowScriptWorkspace
		? []
		: pendingCommands;
	const hasDismissOnlyStaleReview =
		Boolean(pendingFlowIrCommit) &&
		visiblePendingCommands.length === 0 &&
		flowscriptWorkspaceStatus === "stale";
	const pendingJobRequiresExplicitApproval = Boolean(
		pendingBoardEditJob &&
			(pendingBoardEditJob.review.replacementMode ||
				pendingBoardEditJob.review.destructiveEffects.length > 0),
	);

	// Auto mode applies a settled review as soon as generation finishes, mirroring the exact
	// condition that renders the review card. The attempt stamp makes this fire once per
	// distinct review: every early return inside executePendingCommands either leaves the
	// review untouched (key unchanged, so no retry) or clears it outright. Bailing on
	// `destructiveApplyRequest` is load-bearing — cancelling that dialog restores the
	// applicable workspace, and without the bail the effect would immediately re-raise it.
	const autoApplyKey =
		!autoMode ||
		loading ||
		destructiveApplyRequest !== null ||
		pendingJobRequiresExplicitApproval ||
		hasDismissOnlyStaleReview ||
		!(agentMode === "board" || agentMode === "both") ||
		!(
			visiblePendingCommands.length > 0 ||
			hasUnappliedFlowScriptWorkspace ||
			pendingBoardEditJob
		)
			? null
			: [
					pendingFlowIrCommit?.claim_id ?? "",
					pendingBoardEditJob?.jobId ?? "",
					hasUnappliedFlowScriptWorkspace ? flowscriptWorkspace : "",
					visiblePendingCommands
						.map((command) => JSON.stringify(command))
						.join("|"),
				].join("::");

	useEffect(() => {
		if (!autoApplyKey || autoApplyAttemptRef.current === autoApplyKey) return;
		autoApplyAttemptRef.current = autoApplyKey;
		void executePendingCommands();
	}, [autoApplyKey, executePendingCommands]);

	// Components stream in batches during generation, so this waits for `!loading` rather
	// than applying partial batches.
	const autoApplyComponentsKey =
		!autoMode ||
		loading ||
		!(agentMode === "ui" || agentMode === "both") ||
		pendingComponents.length === 0
			? null
			: pendingComponents
					.map((component) => JSON.stringify(component))
					.join("|");

	useEffect(() => {
		if (
			!autoApplyComponentsKey ||
			autoApplyComponentsAttemptRef.current === autoApplyComponentsKey
		)
			return;
		autoApplyComponentsAttemptRef.current = autoApplyComponentsKey;
		handleApplyComponents();
	}, [autoApplyComponentsKey, handleApplyComponents]);

	useEffect(() => {
		onWorkspaceVisibleChange?.(showFlowScriptWorkspace);
	}, [onWorkspaceVisibleChange, showFlowScriptWorkspace]);

	return (
		<motion.div
			layoutId="flowpilot"
			initial={{ opacity: 0, x: 100 }}
			animate={{ opacity: 1, x: 0 }}
			exit={{ opacity: 0, x: 100 }}
			transition={{ type: "spring", stiffness: 400, damping: 30 }}
			className={cn(
				"h-full w-full flex flex-col bg-background/95 backdrop-blur-xl border-l border-border/40 overflow-hidden",
				className,
			)}
		>
			{/* Header */}
			<Header
				title={title}
				loading={loading}
				loadingPhase={loadingPhase}
				loadingStartTime={loadingStartTime}
				runContext={runContext}
				onNewChat={handleNewChat}
				onClose={onClose}
				showHistory={showHistory}
				setShowHistory={setShowHistory}
				// Provider props
				provider={provider}
				onProviderChange={setProvider}
				forceProvider={forceProvider}
				copilotSDK={copilotSDK}
				onStartCopilot={handleStartCopilot}
				onStopCopilot={handleStopCopilot}
				// Model props
				bitsModels={bitsModels}
				selectedModelId={selectedModelId}
				setSelectedModelId={setSelectedModelId}
				selectedReasoningEffort={selectedReasoningEffort}
				setSelectedReasoningEffort={setSelectedReasoningEffort}
				hasWorkspace={hasFlowScriptWorkspace}
				showWorkspace={showWorkspace}
				onToggleWorkspace={() => setShowWorkspace((value) => !value)}
				autoMode={autoMode}
				onToggleAutoMode={handleToggleAutoMode}
			/>

			<div
				className={cn(
					"flex min-h-0 flex-1 overflow-hidden",
					showFlowScriptWorkspace ? "flex-col md:flex-row" : "flex-col",
				)}
			>
				<section className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
					{/* Messages area */}
					<ScrollArea
						className="flex-1 min-h-0 w-full max-w-full overflow-x-hidden px-3"
						viewportRef={scrollContainerRef}
						onScroll={handleScroll}
					>
						<div className="w-full min-w-0 max-w-full space-y-3 overflow-hidden py-3">
							{messages.length === 0 ? (
								<EmptyState
									agentMode={agentMode}
									selectedCount={
										agentMode === "board"
											? selectedNodeIds.length
											: selectedComponentIds.length
									}
									setInput={setInput}
								/>
							) : (
								messages.map((message, index) => {
									const isLastMessage = index === messages.length - 1;
									// Completed (non-last) bubbles get only stable props so their
									// React.memo holds and they don't reconcile (re-parsing markdown)
									// on every streaming flush. Only the live last bubble takes the
									// per-flush loading/step props.
									if (!isLastMessage) {
										return (
											<MessageBubble
												key={index}
												message={message}
												agentMode={agentMode}
												board={board}
												onFocusNode={onFocusNode}
												onSelectNodes={onSelectNodes}
											/>
										);
									}
									const renderedMessage =
										processEvents.length > 0 &&
										(!message.processEvents ||
											message.processEvents.length < processEvents.length)
											? { ...message, processEvents }
											: message;
									return (
										<MessageBubble
											key={index}
											message={renderedMessage}
											isLoading={loading}
											loadingPhase={loadingPhase}
											currentToolCall={currentToolCall}
											currentStep={
												loading
													? planSteps.find((s) => s.status === "InProgress")
													: undefined
											}
											agentMode={agentMode}
											board={board}
											onFocusNode={onFocusNode}
											onSelectNodes={onSelectNodes}
											liveFlowScriptPreview={resolveLiveFlowScriptPreviewForMessage(
												{
													isLatestMessage: isLastMessage,
													messageRole: message.role,
													preview: inlineFlowScriptPreview,
													workspaceStatus: flowscriptWorkspaceStatus,
												},
											)}
										/>
									);
								})
							)}
							<div ref={messagesEndRef} />
						</div>
					</ScrollArea>

					{/* Scroll to bottom indicator */}
					<AnimatePresence>
						{userScrolledUp && messages.length > 0 && (
							<motion.div
								initial={{ opacity: 0, y: 10 }}
								animate={{ opacity: 1, y: 0 }}
								exit={{ opacity: 0, y: 10 }}
								className="absolute bottom-36 left-1/2 z-10 -translate-x-1/2"
							>
								<Button
									size="sm"
									variant="secondary"
									className="h-6 gap-1 rounded-full border border-border/50 px-2 text-[10px] shadow-lg"
									onClick={() => scrollToBottom(true)}
								>
									<ArrowDown className="h-3 w-3" />
									New
								</Button>
							</motion.div>
						)}
					</AnimatePresence>

					{/* Pending commands (board mode or both mode) */}
					{!loading &&
						(agentMode === "board" || agentMode === "both") &&
						(visiblePendingCommands.length > 0 ||
							hasUnappliedFlowScriptWorkspace ||
							Boolean(pendingBoardEditJob) ||
							hasDismissOnlyStaleReview) && (
							<div className="px-3 pb-2">
								<PendingCommandsView
									commands={visiblePendingCommands}
									flowscriptReady={hasUnappliedFlowScriptWorkspace}
									dismissOnly={hasDismissOnlyStaleReview}
									retainedReview={pendingBoardEditJob?.review}
									onExecute={handleExecuteCommands}
									onExecuteSingle={handleExecuteSingle}
									onDismiss={handleDismissCommands}
								/>
							</div>
						)}

					{/* Pending components (UI mode or both mode) */}
					{(agentMode === "ui" || agentMode === "both") &&
						pendingComponents.length > 0 && (
							<PendingComponentsView
								components={pendingComponents}
								canvasSettings={pendingCanvasSettings}
								warnings={validationWarnings}
								onApply={handleApplyComponents}
								onDismiss={handleDismissComponents}
							/>
						)}

					{/* Input area */}
					<div className="shrink-0 border-t border-border/30 bg-background/80 p-2.5 backdrop-blur-sm">
						{/* Context indicator */}
						{contextIndicator}

						{/* Image previews */}
						{attachedImages.length > 0 && (
							<div className="mb-2 flex flex-wrap gap-1.5">
								{attachedImages.map((img, idx) => (
									<div key={idx} className="group relative">
										<img
											src={img.preview}
											alt={`Attached ${idx + 1}`}
											className="h-12 w-12 rounded-md border border-border/50 object-cover"
										/>
										<button
											type="button"
											onClick={() => handleRemoveImage(idx)}
											className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full bg-destructive text-destructive-foreground opacity-0 transition-opacity group-hover:opacity-100"
										>
											<XIcon className="h-2.5 w-2.5" />
										</button>
									</div>
								))}
							</div>
						)}

						<div className="relative flex items-center gap-1.5">
							<input
								type="file"
								ref={imageInputRef}
								accept="image/png,image/jpeg,image/webp,image/gif"
								multiple
								onChange={handleImageSelect}
								className="hidden"
							/>
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										type="button"
										variant="ghost"
										size="icon"
										className="h-10 w-10 shrink-0 rounded-lg hover:bg-accent/50"
										onClick={() => imageInputRef.current?.click()}
										disabled={
											loading || attachedImages.length >= MAX_ATTACHED_IMAGES
										}
									>
										<ImageIcon className="h-4 w-4" />
									</Button>
								</TooltipTrigger>
								<TooltipContent side="top" className="text-xs">
									Attach image (paste supported)
								</TooltipContent>
							</Tooltip>
							<textarea
								value={input}
								onChange={(e) => setInput(e.target.value)}
								onKeyDown={handleKeyDown}
								onPaste={handlePaste}
								placeholder={placeholderText}
								className={cn(
									"max-h-30 min-h-10 flex-1 resize-none rounded-lg border border-border/50 bg-background/80 px-3 py-2.5 text-sm focus-visible:border-primary/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/20",
									captureScreenshot ? "pr-18" : "pr-12",
								)}
								disabled={loading}
								rows={1}
							/>
							{captureScreenshot ? (
								// Split button: Send + dropdown with screenshot option
								<div className="absolute right-1 top-1/2 flex -translate-y-1/2 items-center">
									<Tooltip>
										<TooltipTrigger asChild>
											<Button
												size="icon"
												onClick={() => handleSubmit(false)}
												disabled={
													loading ||
													(!input.trim() && attachedImages.length === 0)
												}
												className="h-8 w-8 rounded-l-lg rounded-r-none bg-linear-to-br from-primary to-purple-600 shadow-md transition-all duration-200 hover:shadow-lg hover:shadow-primary/20 disabled:opacity-50"
											>
												{loading ? (
													<Loader2 className="h-3.5 w-3.5 animate-spin" />
												) : (
													<SendIcon className="h-3.5 w-3.5" />
												)}
											</Button>
										</TooltipTrigger>
										<TooltipContent side="top" className="text-xs">
											Send message
										</TooltipContent>
									</Tooltip>
									<DropdownMenu>
										<DropdownMenuTrigger asChild>
											<Button
												size="icon"
												disabled={
													loading ||
													(!input.trim() && attachedImages.length === 0)
												}
												className="h-8 w-6 rounded-l-none rounded-r-lg border-l border-white/20 bg-linear-to-br from-purple-600 to-pink-600 shadow-md transition-all duration-200 hover:shadow-lg hover:shadow-primary/20 disabled:opacity-50"
											>
												<ChevronDownIcon className="h-3 w-3" />
											</Button>
										</DropdownMenuTrigger>
										<DropdownMenuContent align="end" className="min-w-45">
											<DropdownMenuItem
												onClick={() => handleSubmit(false)}
												disabled={loading}
											>
												<SendIcon className="mr-2 h-4 w-4" />
												Send
											</DropdownMenuItem>
											<DropdownMenuItem
												onClick={() => handleSubmit(true)}
												disabled={loading}
											>
												<CameraIcon className="mr-2 h-4 w-4" />
												Send with screenshot
											</DropdownMenuItem>
										</DropdownMenuContent>
									</DropdownMenu>
								</div>
							) : (
								// Simple send button when no screenshot function
								<Button
									size="icon"
									onClick={() => handleSubmit(false)}
									disabled={
										loading || (!input.trim() && attachedImages.length === 0)
									}
									className="absolute right-1 top-1/2 h-8 w-8 -translate-y-1/2 rounded-lg bg-linear-to-br from-primary to-purple-600 shadow-md transition-all duration-200 hover:shadow-lg hover:shadow-primary/20 disabled:opacity-50"
								>
									{loading ? (
										<Loader2 className="h-3.5 w-3.5 animate-spin" />
									) : (
										<SendIcon className="h-3.5 w-3.5" />
									)}
								</Button>
							)}
						</div>
					</div>
				</section>

				{showFlowScriptWorkspace && (
					<FlowScriptWorkspacePanel
						source={flowscriptWorkspace}
						status={flowscriptWorkspaceStatus}
						onClose={() => setShowWorkspace(false)}
					/>
				)}
			</div>

			<FrontendToolRequestDialog
				dialog={frontendToolDialog}
				onDialogChange={setFrontendToolDialog}
				onResolve={(value) => {
					if (!frontendToolDialog) return;
					resolveFrontendToolDialog(
						frontendToolDialog.request.requestId,
						value,
					);
				}}
			/>

			<Dialog
				open={Boolean(destructiveApplyRequest)}
				onOpenChange={(open) => {
					if (!open && !destructiveApplyPending) {
						setDestructiveApplyRequest(null);
					}
				}}
			>
				<DialogContent className="max-w-md">
					<DialogHeader>
						<DialogTitle>Approve deletion</DialogTitle>
						<DialogDescription>
							Applying this FlowScript needs to delete existing board items
							before it can continue. Deletions are never automatic.
						</DialogDescription>
					</DialogHeader>
					<DialogBody>
						<div className="max-h-36 overflow-y-auto rounded-md border border-destructive/30 bg-destructive/10 p-3 text-xs text-foreground">
							{destructiveApplyRequest?.diagnostic}
						</div>
					</DialogBody>
					<DialogFooter>
						<Button
							variant="secondary"
							disabled={destructiveApplyPending}
							onClick={() => setDestructiveApplyRequest(null)}
						>
							Cancel
						</Button>
						<Button
							variant="destructive"
							className="gap-2"
							disabled={destructiveApplyPending}
							onClick={handleApproveFlowScriptDeletion}
						>
							{destructiveApplyPending ? (
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
							) : null}
							Delete and apply
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			{/* History Panel */}
			<HistoryPanel
				mode={agentMode}
				currentConversationId={currentConversationId}
				onSelectConversation={handleSelectConversation}
				onNewConversation={handleNewChat}
				isOpen={showHistory}
				onClose={() => setShowHistory(false)}
			/>
		</motion.div>
	);
}

export const FlowPilot = memo(FlowPilotImpl);

interface FrontendToolRequestDialogProps {
	dialog: FrontendToolDialogState | null;
	onDialogChange: (dialog: FrontendToolDialogState | null) => void;
	onResolve: (value: unknown) => void;
}

const FrontendToolRequestDialog = memo(function FrontendToolRequestDialog({
	dialog,
	onDialogChange,
	onResolve,
}: FrontendToolRequestDialogProps) {
	if (!dialog) return null;

	const args = dialog.request.arguments;
	const choices = Array.isArray(args.choices)
		? (args.choices as FrontendToolChoice[])
		: [];
	const question =
		getArgString(args, "question") ??
		dialog.request.approval?.title ??
		"FlowPilot request";
	const description =
		dialog.type === "approval"
			? (dialog.request.approval?.description ??
				"Approve this FlowPilot action?")
			: getArgString(args, "description") ||
				getArgString(args, "placeholder") ||
				"Provide the value FlowPilot needs to continue.";

	const resolveAsk = () => {
		if (dialog.type !== "ask") return;
		if (dialog.mode === "freeform") {
			onResolve(dialog.answer);
			return;
		}
		const selectedChoices = Array.from(dialog.selected)
			.sort((a, b) => a - b)
			.map((index) => choices[index])
			.filter(Boolean)
			.map((choice) => choice.value ?? choice.label);
		onResolve(
			dialog.mode === "single_choice"
				? (selectedChoices[0] ?? null)
				: selectedChoices,
		);
	};

	const updateAsk = (
		patch: Partial<Extract<FrontendToolDialogState, { type: "ask" }>>,
	) => {
		if (dialog.type !== "ask") return;
		onDialogChange({ ...dialog, ...patch });
	};

	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open) {
					onResolve(
						dialog.type === "approval"
							? { approved: false, remember: false }
							: null,
					);
				}
			}}
		>
			<DialogContent className="sm:max-w-xl">
				<DialogHeader>
					<DialogTitle>
						{dialog.type === "approval"
							? (dialog.request.approval?.title ?? "Approve FlowPilot action")
							: question}
					</DialogTitle>
					<DialogDescription>{description}</DialogDescription>
				</DialogHeader>
				<DialogBody className="space-y-3">
					{dialog.type === "approval" ? (
						<>
							<div className="rounded-lg border border-border/50 bg-muted/30 p-3">
								<div className="mb-1 text-xs font-medium text-muted-foreground">
									Tool
								</div>
								<div className="font-mono text-xs text-foreground">
									{dialog.request.toolName}
								</div>
							</div>
							<pre className="max-h-56 overflow-auto rounded-lg border border-border/50 bg-background/80 p-3 text-xs text-muted-foreground">
								{JSON.stringify(dialog.request.arguments, null, 2)}
							</pre>
							<label className="flex items-center gap-2 text-sm text-muted-foreground">
								<Checkbox
									checked={dialog.remember}
									onCheckedChange={(checked) =>
										onDialogChange({
											...dialog,
											remember: checked === true,
										})
									}
								/>
								Don&apos;t ask again for this action this session
							</label>
						</>
					) : dialog.mode === "freeform" ? (
						<Textarea
							value={dialog.answer}
							placeholder={
								getArgString(args, "placeholder") ?? "Enter value..."
							}
							className="min-h-28 resize-none"
							onChange={(event) => updateAsk({ answer: event.target.value })}
						/>
					) : (
						<div className="space-y-2">
							{choices.map((choice, index) => {
								const selected = dialog.selected.has(index);
								return (
									<button
										key={`${choice.label}-${index}`}
										type="button"
										className={cn(
											"flex w-full items-start gap-3 rounded-lg border p-3 text-left transition-colors",
											selected
												? "border-primary/50 bg-primary/10"
												: "border-border/50 bg-background/70 hover:bg-muted/40",
										)}
										onClick={() => {
											const next = new Set(dialog.selected);
											if (dialog.mode === "single_choice") {
												next.clear();
												next.add(index);
											} else if (next.has(index)) {
												next.delete(index);
											} else {
												next.add(index);
											}
											updateAsk({ selected: next });
										}}
									>
										<Checkbox checked={selected} className="mt-0.5" />
										<div className="min-w-0">
											<div className="text-sm font-medium">{choice.label}</div>
											{choice.description && (
												<div className="mt-0.5 text-xs text-muted-foreground">
													{choice.description}
												</div>
											)}
										</div>
									</button>
								);
							})}
						</div>
					)}
				</DialogBody>
				<DialogFooter>
					<Button
						type="button"
						variant="outline"
						onClick={() =>
							onResolve(
								dialog.type === "approval"
									? { approved: false, remember: false }
									: null,
							)
						}
					>
						{dialog.type === "approval" ? "Deny" : "Cancel"}
					</Button>
					<Button
						type="button"
						onClick={() => {
							if (dialog.type === "approval") {
								onResolve({ approved: true, remember: dialog.remember });
							} else {
								resolveAsk();
							}
						}}
					>
						{dialog.type === "approval" ? "Approve" : "Submit"}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
});

// Self-contained elapsed-time pill: owns the 1s tick so it does NOT re-render the
// whole FlowPilot tree every second during loading.
const ElapsedStatusPill = memo(function ElapsedStatusPill({
	phase,
	loadingStartTime,
	compact,
}: {
	phase: LoadingPhase;
	loadingStartTime: number | null;
	compact?: boolean;
}) {
	const [elapsed, setElapsed] = useState(() =>
		loadingStartTime ? Math.floor((Date.now() - loadingStartTime) / 1000) : 0,
	);
	useEffect(() => {
		if (!loadingStartTime) {
			setElapsed(0);
			return;
		}
		setElapsed(Math.floor((Date.now() - loadingStartTime) / 1000));
		const interval = setInterval(() => {
			setElapsed(Math.floor((Date.now() - loadingStartTime) / 1000));
		}, 1000);
		return () => clearInterval(interval);
	}, [loadingStartTime]);
	return <StatusPill phase={phase} elapsed={elapsed} compact={compact} />;
});

// Header component
interface HeaderProps {
	title: string;
	loading: boolean;
	loadingPhase: LoadingPhase;
	loadingStartTime: number | null;
	runContext?: { run_id: string };
	onNewChat: () => void;
	onClose?: () => void;
	// History props
	showHistory: boolean;
	setShowHistory: (show: boolean) => void;
	// Provider props
	provider: AIProvider;
	onProviderChange: (provider: AIProvider) => void;
	forceProvider?: AIProvider;
	copilotSDK: ReturnType<typeof useCopilotSDK>;
	onStartCopilot: (
		backend?: AgentBackendProvider,
		serverUrl?: string,
	) => Promise<void>;
	onStopCopilot: () => Promise<void>;
	// Model props
	bitsModels: any[];
	selectedModelId: string;
	setSelectedModelId: (id: string) => void;
	selectedReasoningEffort: string;
	setSelectedReasoningEffort: (effort: string) => void;
	hasWorkspace: boolean;
	showWorkspace: boolean;
	onToggleWorkspace: () => void;
	autoMode: boolean;
	onToggleAutoMode: () => void;
}

const Header = memo(function Header({
	title,
	loading,
	loadingPhase,
	loadingStartTime,
	runContext,
	onNewChat,
	onClose,
	showHistory,
	setShowHistory,
	provider,
	onProviderChange,
	forceProvider,
	copilotSDK,
	onStartCopilot,
	onStopCopilot,
	bitsModels,
	selectedModelId,
	setSelectedModelId,
	selectedReasoningEffort,
	setSelectedReasoningEffort,
	hasWorkspace,
	showWorkspace,
	onToggleWorkspace,
	autoMode,
	onToggleAutoMode,
}: HeaderProps) {
	const normalizedProvider = normalizeAIProvider(provider);
	const pickerProviders: ProviderModelPickerProvider[] = [
		{ id: "bits", label: "Bits", title: "Use configured model bits" },
		{
			id: "github-copilot",
			label: "Copilot",
			title: "Use GitHub Copilot SDK (local)",
			disabled: !isTauriRuntime(),
		},
		{
			id: "codex",
			label: "Codex",
			title: "Use a tool-capable Codex backend adapter",
			disabled: !isTauriRuntime(),
		},
		{
			id: "claude-code",
			label: "Claude Code",
			title: "Use the Claude Code CLI through the shared FlowPilot MCP tools",
			disabled: !isTauriRuntime(),
		},
	];
	const pickerModels =
		normalizedProvider === "bits"
			? bitsModels.map((model) => ({
					id: model.id as string,
					label:
						model.meta?.en?.name ?? model.friendly_name ?? (model.id as string),
					isFree: isFreeLlmModel(model),
				}))
			: copilotSDK.models.map((model) => ({
					id: model.id,
					label: model.name || model.id,
					supportedReasoningEfforts: model.supportedReasoningEfforts,
					defaultReasoningEffort: model.defaultReasoningEffort,
				}));
	const handlePickerProviderChange = useCallback(
		async (nextProvider: AIProvider) => {
			const normalized = normalizeAIProvider(nextProvider);
			onProviderChange(normalized);
			if (!isAgentBackendProvider(normalized) || !isTauriRuntime()) return;
			if (
				copilotSDK.isRunning &&
				normalized === normalizeAIProvider(provider)
			) {
				return;
			}
			try {
				await onStartCopilot(normalized);
			} catch {
				// The SDK hook surfaces connection failures in its own status.
			}
		},
		[copilotSDK.isRunning, onProviderChange, onStartCopilot, provider],
	);
	const connectionStatus =
		isAgentBackendProvider(normalizedProvider) && !copilotSDK.diagnostic
			? copilotSDK.authStatus?.authenticated && copilotSDK.authStatus.login
				? `Signed in as ${copilotSDK.authStatus.login}${copilotSDK.authStatus.message ? ` · ${copilotSDK.authStatus.message}` : ""}`
				: copilotSDK.authStatus?.message
			: undefined;

	return (
		<div className="relative overflow-hidden shrink-0">
			<div className="absolute inset-0 bg-linear-to-br from-primary/8 via-violet-500/5 to-pink-500/5" />
			{loading && (
				<motion.div
					className="absolute inset-0 opacity-30"
					style={{
						background:
							"radial-gradient(circle at 30% 50%, rgba(139, 92, 246, 0.3), transparent 50%), radial-gradient(circle at 70% 50%, rgba(236, 72, 153, 0.3), transparent 50%)",
					}}
					animate={{
						background: [
							"radial-gradient(circle at 30% 50%, rgba(139, 92, 246, 0.3), transparent 50%), radial-gradient(circle at 70% 50%, rgba(236, 72, 153, 0.3), transparent 50%)",
							"radial-gradient(circle at 70% 50%, rgba(139, 92, 246, 0.3), transparent 50%), radial-gradient(circle at 30% 50%, rgba(236, 72, 153, 0.3), transparent 50%)",
						],
					}}
					transition={{
						duration: 3,
						repeat: Number.POSITIVE_INFINITY,
						repeatType: "reverse",
					}}
				/>
			)}

			<div className="relative px-3 py-2.5 flex items-center justify-between">
				<div className="flex items-center gap-2">
					<div className="relative">
						<motion.div
							className="absolute inset-0 bg-linear-to-br from-primary to-violet-600 rounded-lg blur-md opacity-50"
							animate={
								loading ? { scale: [1, 1.2, 1], opacity: [0.5, 0.8, 0.5] } : {}
							}
							transition={{ duration: 2, repeat: Number.POSITIVE_INFINITY }}
						/>
						<div className="relative p-1.5 bg-linear-to-br from-primary via-violet-600 to-pink-600 rounded-lg shadow-md">
							<SparklesIcon className="w-3.5 h-3.5 text-white" />
						</div>
					</div>
					<div>
						<h3 className="text-sm font-bold">{title}</h3>
						{loading ? (
							<ElapsedStatusPill
								phase={loadingPhase}
								loadingStartTime={loadingStartTime}
								compact
							/>
						) : (
							<div className="flex items-center gap-1 text-xs text-muted-foreground">
								<span className="relative flex h-1.5 w-1.5">
									<span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75" />
									<span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-green-500" />
								</span>
								{runContext ? "Log context active" : "Ready"}
							</div>
						)}
					</div>
				</div>
				<div className="flex items-center gap-2">
					{hasWorkspace && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant={showWorkspace ? "secondary" : "ghost"}
									size="icon"
									className="h-7 w-7 rounded-md hover:bg-accent/50"
									onClick={onToggleWorkspace}
								>
									<FileCode2Icon className="w-4 h-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent side="bottom" className="text-xs">
								FlowScript workspace
							</TooltipContent>
						</Tooltip>
					)}
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant={autoMode ? "secondary" : "ghost"}
								size="icon"
								aria-pressed={autoMode}
								className="h-7 w-7 rounded-md hover:bg-accent/50"
								onClick={onToggleAutoMode}
							>
								<ZapIcon className="w-4 h-4" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="bottom" className="text-xs">
							{autoMode
								? "Auto mode on — tools run and changes apply without asking, including destructive ones. Only board-item deletion still asks."
								: "Auto mode off — FlowPilot asks before acting"}
						</TooltipContent>
					</Tooltip>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-7 w-7 rounded-md hover:bg-accent/50"
								onClick={() => setShowHistory(!showHistory)}
							>
								<ClockIcon className="w-4 h-4" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="bottom" className="text-xs">
							History
						</TooltipContent>
					</Tooltip>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-7 w-7 rounded-md hover:bg-accent/50"
								onClick={onNewChat}
							>
								<SquarePenIcon className="w-4 h-4" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="bottom" className="text-xs">
							New chat
						</TooltipContent>
					</Tooltip>
					{onClose && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-7 w-7 rounded-md hover:bg-accent/50"
									onClick={onClose}
								>
									<XIcon className="w-4 h-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent side="bottom" className="text-xs">
								Close
							</TooltipContent>
						</Tooltip>
					)}
				</div>
			</div>

			{/* Provider, model, and model-native effort are one selection surface. */}
			<div className="relative px-3 pb-3">
				<ProviderModelReasoningPicker
					provider={provider}
					providers={pickerProviders}
					models={pickerModels}
					selectedModelId={selectedModelId}
					selectedEffort={selectedReasoningEffort}
					onProviderChange={handlePickerProviderChange}
					onModelChange={setSelectedModelId}
					onEffortChange={setSelectedReasoningEffort}
					disabled={loading}
					connecting={copilotSDK.isConnecting}
					connected={
						copilotSDK.isRunning && isAgentBackendProvider(normalizedProvider)
					}
					diagnostic={
						isAgentBackendProvider(normalizedProvider)
							? copilotSDK.diagnostic
							: null
					}
					onRetry={copilotSDK.retry}
					onDisconnect={onStopCopilot}
					statusText={connectionStatus}
					showProviderSection={!forceProvider}
					triggerClassName="w-full justify-start md:w-auto md:max-w-full"
					contentClassName="z-150"
					emptyModelLabel={
						normalizedProvider === "bits"
							? "No models available"
							: "Loading backend models…"
					}
				/>
			</div>

			{loading && (
				<motion.div
					className="absolute bottom-0 left-0 right-0 h-0.5 bg-muted/30"
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
				>
					<motion.div
						className="h-full bg-linear-to-r from-primary via-violet-500 to-pink-500"
						initial={{ width: "0%" }}
						animate={{ width: "100%" }}
						transition={{ duration: 30, ease: "linear" }}
					/>
				</motion.div>
			)}
		</div>
	);
});

// Empty state component
interface EmptyStateProps {
	agentMode: AgentMode;
	selectedCount: number;
	setInput: (v: string) => void;
}

const EmptyState = memo(function EmptyState({
	agentMode,
	selectedCount,
	setInput,
}: EmptyStateProps) {
	const suggestions = useMemo(() => {
		if (agentMode === "both") {
			return selectedCount > 0
				? ["Explain this", "Create UI for it", "Add workflow step"]
				: [
						"Create a dashboard with API",
						"Build form + workflow",
						"Design UI with data flow",
					];
		}
		if (agentMode === "board") {
			return selectedCount > 0
				? ["Explain this node", "Connect to output", "Add error handling"]
				: ["Create a REST API node", "Build a data pipeline", "Add logging"];
		}
		return selectedCount > 0
			? ["Make it larger", "Change the color", "Add a border"]
			: ["Create a login form", "Build a card component", "Design a navbar"];
	}, [agentMode, selectedCount]);

	const description = useMemo(() => {
		if (selectedCount > 0) {
			if (agentMode === "both")
				return "Describe what to do with the selected items";
			return `Describe what to do with the selected ${agentMode === "board" ? "nodes" : "components"}`;
		}
		if (agentMode === "both") return "Build workflows, UIs, or both together";
		if (agentMode === "board") return "Ask questions or build your flow";
		return "Describe the UI you want to create";
	}, [agentMode, selectedCount]);

	return (
		<div className="flex flex-col items-center justify-center py-8 text-center">
			<motion.div
				initial={{ scale: 0 }}
				animate={{ scale: 1 }}
				transition={{ type: "spring", stiffness: 400, damping: 20 }}
			>
				<div className="relative">
					<motion.div
						className="absolute inset-0 bg-linear-to-br from-primary/30 to-violet-500/30 rounded-full blur-xl"
						animate={{ scale: [1, 1.2, 1], opacity: [0.5, 0.8, 0.5] }}
						transition={{ duration: 3, repeat: Number.POSITIVE_INFINITY }}
					/>
					<SparklesIcon className="w-12 h-12 relative text-primary/50" />
				</div>
			</motion.div>
			<p className="text-sm font-medium text-foreground mt-3 mb-1">
				How can I help?
			</p>
			<p className="text-xs text-muted-foreground max-w-50">{description}</p>
			<div className="flex flex-wrap gap-2 justify-center pt-4">
				{suggestions.map((suggestion, i) => (
					<motion.button
						key={suggestion}
						initial={{ opacity: 0, y: 5 }}
						animate={{ opacity: 1, y: 0 }}
						transition={{ delay: 0.1 * i }}
						onClick={() => setInput(suggestion)}
						className="px-3 py-1.5 text-xs rounded-full bg-muted/50 hover:bg-muted text-muted-foreground hover:text-foreground transition-colors"
					>
						{suggestion}
					</motion.button>
				))}
			</div>
		</div>
	);
});

interface ProcessTimelineProps {
	events: FlowPilotProcessEvent[];
	className?: string;
}

const ProcessTimeline = memo(function ProcessTimeline({
	events,
	className,
}: ProcessTimelineProps) {
	const scrollRef = useRef<HTMLDivElement>(null);
	const shouldStickToBottomRef = useRef(true);
	const completed = events.filter((event) => event.status === "done").length;
	const running = events.some((event) => event.status === "running");
	const failed = events.filter((event) => event.status === "error").length;
	const progress = Math.round((completed / Math.max(events.length, 1)) * 100);
	const activeEvent =
		[...events].reverse().find((event) => event.status === "running") ??
		events[events.length - 1];
	const activeSummary = activeEvent
		? [activeEvent.title, activeEvent.summary].filter(Boolean).join(" - ")
		: undefined;
	const stateLabel =
		failed > 0 ? "Needs attention" : running ? "Working" : "Complete";

	const handleProcessScroll = useCallback(() => {
		const container = scrollRef.current;
		if (!container) return;
		const distanceFromBottom =
			container.scrollHeight - container.scrollTop - container.clientHeight;
		shouldStickToBottomRef.current = distanceFromBottom < 32;
	}, []);

	useEffect(() => {
		if (!shouldStickToBottomRef.current) return;
		const container = scrollRef.current;
		if (!container) return;

		const frame = requestAnimationFrame(() => {
			container.scrollTop = container.scrollHeight;
		});

		return () => cancelAnimationFrame(frame);
	}, [events]);

	if (events.length === 0) return null;

	return (
		<div
			className={cn(
				"relative w-full min-w-0 max-w-full overflow-hidden rounded-xl border border-border/35 bg-background/70 shadow-sm shadow-black/5",
				className,
			)}
		>
			<div className="absolute inset-x-0 top-0 h-px bg-linear-to-r from-transparent via-primary/35 to-transparent" />
			<div className="border-b border-border/30 px-2.5 py-2">
				<div className="flex w-full min-w-0 items-center justify-between gap-2 overflow-hidden">
					<div className="flex min-w-0 flex-1 items-center gap-2 overflow-hidden">
						<div
							className={cn(
								"flex h-6 w-6 shrink-0 items-center justify-center rounded-md border",
								running
									? "border-primary/25 bg-primary/10 text-primary"
									: failed > 0
										? "border-destructive/25 bg-destructive/10 text-destructive"
										: "border-green-500/25 bg-green-500/10 text-green-600",
							)}
						>
							{running ? (
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
							) : failed > 0 ? (
								<XIcon className="h-3.5 w-3.5" />
							) : (
								<CheckCircle2 className="h-3.5 w-3.5" />
							)}
						</div>
						<div className="min-w-0 flex-1 overflow-hidden">
							<div className="flex max-w-full min-w-0 items-center gap-1.5 overflow-hidden">
								<ListTreeIcon className="h-3 w-3 shrink-0 text-muted-foreground" />
								<span className="truncate text-[11px] font-semibold text-foreground">
									Agent Process
								</span>
								<span
									className={cn(
										"shrink-0 rounded-full border px-1.5 py-0.5 text-[9px] font-medium",
										running
											? "border-primary/25 bg-primary/10 text-primary"
											: failed > 0
												? "border-destructive/25 bg-destructive/10 text-destructive"
												: "border-green-500/25 bg-green-500/10 text-green-600",
									)}
								>
									{stateLabel}
								</span>
							</div>
							{activeSummary && (
								<div className="mt-0.5 block max-w-full overflow-hidden text-ellipsis whitespace-nowrap text-[10px] text-muted-foreground">
									{activeSummary}
								</div>
							)}
						</div>
					</div>
					<div className="flex shrink-0 items-center gap-1.5 text-[10px] text-muted-foreground">
						<span>
							{completed}/{events.length}
						</span>
					</div>
				</div>
				<div className="mt-2 h-1 overflow-hidden rounded-full bg-muted">
					<div
						className={cn(
							"h-full rounded-full transition-all duration-300",
							failed > 0 ? "bg-destructive" : "bg-primary",
						)}
						style={{ width: `${progress}%` }}
					/>
				</div>
			</div>
			<div
				ref={scrollRef}
				onScroll={handleProcessScroll}
				className="max-h-72 min-w-0 max-w-full overflow-y-auto overflow-x-hidden px-1.5 py-1"
			>
				{events.map((event, index) => (
					<ProcessTimelineRow
						key={event.id}
						event={event}
						isLast={index === events.length - 1}
					/>
				))}
			</div>
		</div>
	);
});

interface ProcessTimelineRowProps {
	event: FlowPilotProcessEvent;
	isLast: boolean;
}

const ProcessTimelineRow = memo(function ProcessTimelineRow({
	event,
	isLast,
}: ProcessTimelineRowProps) {
	const [open, setOpen] = useState(false);
	const hasDetails = Boolean(
		event.details ||
			event.resultPreview ||
			event.workspaceAfter ||
			(event.commands && event.commands.length > 0),
	);
	const diffLines = useMemo(
		() =>
			event.workspaceAfter
				? buildFlowScriptDiff(event.workspaceBefore, event.workspaceAfter)
				: [],
		[event.workspaceBefore, event.workspaceAfter],
	);
	const elapsed = formatProcessElapsed(event);
	const statusLabel =
		event.status === "running"
			? "Running"
			: event.status === "error"
				? "Error"
				: event.status === "done"
					? "Done"
					: "Info";

	const icon = useMemo(() => {
		if (event.status === "running") {
			return <Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />;
		}
		if (event.status === "error") {
			return <XIcon className="h-3.5 w-3.5 text-destructive" />;
		}
		switch (event.kind) {
			case "workspace":
				return <FileDiffIcon className="h-3.5 w-3.5 text-blue-500" />;
			case "commands":
				return <WorkflowIcon className="h-3.5 w-3.5 text-green-600" />;
			case "components":
				return <LayoutGridIcon className="h-3.5 w-3.5 text-violet-500" />;
			case "progress":
				return <CircleDashedIcon className="h-3.5 w-3.5 text-primary" />;
			default:
				return <WrenchIcon className="h-3.5 w-3.5 text-muted-foreground" />;
		}
	}, [event.kind, event.status]);

	return (
		<div className="relative min-w-0 max-w-full overflow-hidden pl-5">
			{!isLast && (
				<div className="absolute left-[7px] top-7 bottom-0 w-px bg-border/55" />
			)}
			<Collapsible open={open} onOpenChange={setOpen}>
				<div
					className={cn(
						"relative my-0.5 flex w-full min-w-0 max-w-full items-start gap-2 overflow-hidden rounded-lg border px-2 py-2 transition-colors",
						event.status === "running"
							? "border-primary/20 bg-primary/5"
							: event.status === "error"
								? "border-destructive/25 bg-destructive/5"
								: "border-transparent hover:border-border/30 hover:bg-muted/25",
					)}
				>
					<div
						className={cn(
							"absolute left-[-19px] top-2.5 flex h-4 w-4 items-center justify-center rounded-full border bg-background",
							event.status === "running"
								? "border-primary/30 ring-2 ring-primary/10"
								: event.status === "error"
									? "border-destructive/30"
									: "border-border/70",
						)}
					>
						{icon}
					</div>
					<div className="min-w-0 max-w-full flex-1 overflow-hidden">
						<div className="flex max-w-full min-w-0 items-start justify-between gap-2 overflow-hidden">
							<div className="min-w-0 flex-1 overflow-hidden">
								<div className="flex max-w-full min-w-0 items-center gap-1.5 overflow-hidden">
									<div className="min-w-0 flex-1 truncate text-[11px] font-semibold text-foreground">
										{event.title}
									</div>
									<span
										className={cn(
											"shrink-0 rounded-full border px-1.5 py-0.5 text-[9px] font-medium",
											event.status === "running"
												? "border-primary/25 bg-primary/10 text-primary"
												: event.status === "error"
													? "border-destructive/25 bg-destructive/10 text-destructive"
													: event.status === "done"
														? "border-green-500/20 bg-green-500/10 text-green-600"
														: "border-border/50 bg-muted/40 text-muted-foreground",
										)}
									>
										{statusLabel}
									</span>
								</div>
								{event.summary && (
									<div
										className="mt-1 line-clamp-2 max-w-full break-words text-[10px] leading-4 text-muted-foreground"
										style={{ overflowWrap: "anywhere" }}
									>
										{event.summary}
									</div>
								)}
								<div className="mt-1 flex min-w-0 max-w-full flex-wrap items-center gap-1.5 overflow-hidden text-[9px] text-muted-foreground">
									{event.toolName && (
										<span className="min-w-0 max-w-40 truncate rounded bg-muted/55 px-1.5 py-0.5 font-mono">
											{event.toolName}
										</span>
									)}
									{elapsed && <span>{elapsed}</span>}
									{event.commands && event.commands.length > 0 && (
										<span>{event.commands.length} board changes</span>
									)}
									{event.workspaceAfter && (
										<span>{formatLineCount(event.workspaceAfter)}</span>
									)}
								</div>
							</div>
							{hasDetails && (
								<CollapsibleTrigger asChild>
									<Button
										type="button"
										variant="ghost"
										size="icon"
										className="h-5 w-5 shrink-0 rounded-md"
									>
										<ChevronDownIcon
											className={cn(
												"h-3 w-3 transition-transform",
												open && "rotate-180",
											)}
										/>
									</Button>
								</CollapsibleTrigger>
							)}
						</div>
						{hasDetails && (
							<CollapsibleContent>
								<div className="mt-1.5 min-w-0 max-w-full space-y-1.5 overflow-hidden">
									{event.details && (
										<ProcessDetailBlock title="Input" value={event.details} />
									)}
									{event.resultPreview && (
										<ProcessDetailBlock
											title="Result"
											value={event.resultPreview}
										/>
									)}
									{event.commands && event.commands.length > 0 && (
										<div className="min-w-0 max-w-full space-y-1 overflow-hidden rounded-md border border-border/40 bg-background/70 p-2">
											{event.commands.slice(0, 12).map((command, index) => (
												<div
													key={index}
													className="flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground"
												>
													<span className="shrink-0 font-mono text-green-600">
														+
													</span>
													<span className="min-w-0 flex-1 truncate">
														{getCommandSummary(command)}
													</span>
												</div>
											))}
											{event.commands.length > 12 && (
												<div className="text-[10px] text-muted-foreground">
													+ {event.commands.length - 12} more
												</div>
											)}
										</div>
									)}
									{diffLines.length > 0 && (
										<FlowScriptDiffPreview lines={diffLines} />
									)}
								</div>
							</CollapsibleContent>
						)}
					</div>
				</div>
			</Collapsible>
		</div>
	);
});

interface ProcessDetailBlockProps {
	title: string;
	value: string;
}

const ProcessDetailBlock = memo(function ProcessDetailBlock({
	title,
	value,
}: ProcessDetailBlockProps) {
	return (
		<div className="min-w-0 max-w-full overflow-hidden rounded-md border border-border/40 bg-background/70">
			<div className="flex items-center justify-between border-b border-border/30 px-2 py-1 text-[9px] font-medium uppercase tracking-wide text-muted-foreground">
				<span>{title}</span>
			</div>
			<pre className="max-h-36 min-w-0 max-w-full overflow-auto whitespace-pre-wrap break-words p-2 text-[10px] leading-4 text-muted-foreground">
				{value}
			</pre>
		</div>
	);
});

interface FlowScriptDiffPreviewProps {
	lines: FlowScriptDiffLine[];
}

const FlowScriptDiffPreview = memo(function FlowScriptDiffPreview({
	lines,
}: FlowScriptDiffPreviewProps) {
	const added = lines.filter((line) => line.type === "added").length;
	const removed = lines.filter((line) => line.type === "removed").length;

	return (
		<div className="min-w-0 max-w-full overflow-hidden rounded-md border border-border/40 bg-background/80">
			<div className="flex min-w-0 items-center justify-between border-b border-border/30 px-2 py-1">
				<span className="truncate text-[9px] font-medium uppercase tracking-wide text-muted-foreground">
					FlowScript diff
				</span>
				<span className="shrink-0 text-[9px] text-muted-foreground">
					+{added} / -{removed}
				</span>
			</div>
			<pre className="max-h-48 min-w-0 max-w-full overflow-auto p-2 font-mono text-[10px] leading-4">
				{lines.map((line, index) => {
					const prefix =
						line.type === "added" ? "+" : line.type === "removed" ? "-" : " ";
					return (
						<div
							key={`${index}-${line.type}`}
							className={cn(
								"grid grid-cols-[1rem_minmax(0,1fr)] gap-1 whitespace-pre-wrap break-words",
								line.type === "added" &&
									"bg-green-500/10 text-green-700 dark:text-green-300",
								line.type === "removed" &&
									"bg-red-500/10 text-red-700 dark:text-red-300",
								line.type === "context" && "text-muted-foreground",
							)}
						>
							<span className="select-none text-muted-foreground">
								{prefix}
							</span>
							<span>{line.text || " "}</span>
						</div>
					);
				})}
			</pre>
		</div>
	);
});

// Message bubble component
interface MessageBubbleProps {
	message: CopilotMessage;
	isLoading?: boolean;
	loadingPhase?: LoadingPhase;
	currentToolCall?: string | null;
	currentStep?: UnifiedPlanStep;
	agentMode: AgentMode;
	board?: any;
	onFocusNode?: (nodeId: string) => void;
	onSelectNodes?: (nodeIds: string[]) => void;
	liveFlowScriptPreview?: InlineFlowScriptPreviewValue;
}

const MessageBubble = memo(function MessageBubble({
	message,
	isLoading,
	loadingPhase,
	currentToolCall,
	currentStep,
	agentMode,
	board,
	onFocusNode,
	onSelectNodes,
	liveFlowScriptPreview,
}: MessageBubbleProps) {
	const isUser = message.role === "user";
	const hasProcessEvents =
		!isUser && message.processEvents && message.processEvents.length > 0;
	const displayedFlowScriptPreview = resolveDisplayedFlowScriptPreview({
		messageRole: message.role,
		livePreview: liveFlowScriptPreview,
		messageWorkspace: message.flowscriptWorkspace,
	});
	const hasFlowScriptPreview = Boolean(
		displayedFlowScriptPreview?.source.trim(),
	);

	const getLoadingContent = () => {
		// Show current step with details
		if (currentStep) {
			const hasContent = message.content && message.content.trim().length > 0;
			return (
				<div
					className={`space-y-1.5 ${hasContent ? "mt-3 pt-2 border-t border-border/30" : ""}`}
				>
					<div className="flex items-center gap-2">
						<Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
						<span className="text-xs font-medium text-foreground">
							{currentStep.tool_name === "think" ||
							currentStep.tool_name === "analyze"
								? "Thinking"
								: currentStep.tool_name === "emit_surface"
									? "Generating UI"
									: currentStep.tool_name === "emit_commands" ||
											currentStep.tool_name === "write_flowscript" ||
											currentStep.tool_name === "patch_flowscript" ||
											currentStep.tool_name === "check_flowscript" ||
											currentStep.tool_name === "commit_flowscript" ||
											currentStep.tool_name === "edit_flowscript"
										? "Building FlowScript"
										: currentStep.tool_name === "get_component_schema"
											? "Looking up schema"
											: currentStep.tool_name === "get_style_examples"
												? "Fetching styles"
												: currentStep.tool_name?.replace(/_/g, " ") ||
													"Processing"}
						</span>
					</div>
					{currentStep.description && (
						<p className="text-xs text-muted-foreground pl-5 whitespace-pre-wrap line-clamp-4">
							{currentStep.description}
						</p>
					)}
				</div>
			);
		}

		// Fallback to phase-based loading
		const hasContent = message.content && message.content.trim().length > 0;
		return (
			<div
				className={`flex items-center gap-2 ${hasContent ? "mt-3 pt-2 border-t border-border/30" : ""}`}
			>
				<Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
				<span className="text-xs text-muted-foreground">
					{currentToolCall
						? `Using ${currentToolCall.replace(/_/g, " ")}...`
						: loadingPhase && loadingPhase !== "idle"
							? loadingPhase.charAt(0).toUpperCase() +
								loadingPhase.slice(1) +
								"..."
							: "Processing..."}
				</span>
			</div>
		);
	};

	return (
		<motion.div
			initial={{ opacity: 0, y: 10 }}
			animate={{ opacity: 1, y: 0 }}
			className={cn(
				"flex w-full min-w-0 max-w-full overflow-hidden",
				isUser ? "justify-end" : "justify-start",
			)}
		>
			<div
				className={cn(
					"box-border min-w-0 overflow-hidden rounded-xl px-3 py-2 text-sm",
					hasProcessEvents || hasFlowScriptPreview
						? "w-full max-w-full"
						: "max-w-[85%]",
					isUser
						? "bg-muted/60 text-foreground rounded-br-sm border border-border/40"
						: "bg-background border border-border/40 rounded-bl-sm",
				)}
				style={{
					wordBreak: "break-word",
					overflowWrap: "anywhere",
					contain:
						hasProcessEvents || hasFlowScriptPreview
							? "inline-size"
							: undefined,
				}}
			>
				{/* Images */}
				{message.images && message.images.length > 0 && (
					<div className="flex gap-1.5 mb-2 flex-wrap">
						{message.images.map((img, idx) => (
							<img
								key={idx}
								src={img.preview}
								alt={`Attached ${idx + 1}`}
								className="h-16 rounded-md"
							/>
						))}
					</div>
				)}

				{/* Context nodes (board mode or both mode, user messages) */}
				{isUser &&
					(agentMode === "board" || agentMode === "both") &&
					message.contextNodeIds &&
					message.contextNodeIds.length > 0 && (
						<ContextNodes
							nodeIds={message.contextNodeIds}
							board={board}
							onSelectNodes={onSelectNodes}
							onFocusNode={onFocusNode}
							compact
						/>
					)}

				{/* Content. While this bubble is actively streaming, render plain text
				    to avoid re-parsing the full markdown/Slate document on every 100ms
				    flush; the formatted markdown mounts once when streaming completes. */}
				{message.content ? (
					<MessageContent
						content={message.content}
						onFocusNode={onFocusNode}
						board={
							agentMode === "board" || agentMode === "both" ? board : undefined
						}
						enableMarkdown={!isLoading}
					/>
				) : isLoading || hasProcessEvents || hasFlowScriptPreview ? null : (
					<p className="text-muted-foreground italic text-xs">No response</p>
				)}

				{displayedFlowScriptPreview && hasFlowScriptPreview && (
					<InlineFlowScriptPreview preview={displayedFlowScriptPreview} />
				)}

				{hasProcessEvents && (
					<ProcessTimeline
						events={message.processEvents ?? []}
						className={message.content ? "mt-3" : undefined}
					/>
				)}

				{/* Loading indicator - only use the compact fallback when no process timeline is available. */}
				{isLoading && !hasProcessEvents && getLoadingContent()}

				{/* Applied components badge (UI mode) */}
				{message.appliedComponents && message.appliedComponents.length > 0 && (
					<div className="mt-2 flex items-center gap-1 text-green-600 text-xs">
						<CheckCircle2 className="w-3 h-3" />
						<span>{message.appliedComponents.length} components applied</span>
					</div>
				)}

				{/* Executed commands badge (board mode) */}
				{message.executedCommands && message.executedCommands.length > 0 && (
					<div className="mt-2 flex items-center gap-1 text-green-600 text-xs">
						<CheckCircle2 className="w-3 h-3" />
						<span>{message.executedCommands.length} changes applied</span>
					</div>
				)}
			</div>
		</motion.div>
	);
});
