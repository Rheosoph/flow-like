"use client";

import Editor, { type Monaco } from "@monaco-editor/react";
import { AnimatePresence, motion } from "framer-motion";
import {
	ArrowDown,
	CameraIcon,
	CheckCircle2,
	ChevronDownIcon,
	CircleDashedIcon,
	ClockIcon,
	CopyIcon,
	FileCode2Icon,
	FileDiffIcon,
	ImageIcon,
	LayoutGridIcon,
	ListTreeIcon,
	Loader2,
	SendIcon,
	SparklesIcon,
	SquarePenIcon,
	WrenchIcon,
	WorkflowIcon,
	XIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useCopilotSDK, useInvoke } from "../../hooks";
import { IBitTypes } from "../../lib";
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
import { IIndexType } from "../../state/backend-state/db-state";
import { useExecutionServiceOptional } from "../../state/execution-service-context";

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
import { ModelSelector, ProviderSelector } from "./ProviderSelector";
import { StatusPill } from "./StatusPill";
import { buildBudgetedHistory } from "./history-budget";
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
	NormalizedAIProvider,
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
	CanvasSettings,
	CopilotScope,
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
const FLOWSCRIPT_LANGUAGE_ID = "flowscript";
let flowScriptLanguageRegistered = false;

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
	return Array.isArray(boardCommands)
		? (boardCommands as BoardCommand[])
		: [];
}

function destructiveFlowScriptDiagnostic(diagnostics: string[]): string | null {
	return (
		diagnostics.find((diagnostic) =>
			diagnostic.startsWith(DESTRUCTIVE_FLOWSCRIPT_DIAGNOSTIC_PREFIX),
		) ?? null
	);
}

function registerFlowScriptLanguage(monaco: Monaco, isDark: boolean) {
	const hasRegisteredLanguage = monaco.languages
		.getLanguages()
		.some((language) => language.id === FLOWSCRIPT_LANGUAGE_ID);

	if (!flowScriptLanguageRegistered && !hasRegisteredLanguage) {
		monaco.languages.register({ id: FLOWSCRIPT_LANGUAGE_ID });
		monaco.languages.setLanguageConfiguration(FLOWSCRIPT_LANGUAGE_ID, {
			comments: { lineComment: "//" },
			brackets: [
				["{", "}"],
				["[", "]"],
				["(", ")"],
			],
			autoClosingPairs: [
				{ open: "{", close: "}" },
				{ open: "[", close: "]" },
				{ open: "(", close: ")" },
				{ open: '"', close: '"', notIn: ["string", "comment"] },
			],
			surroundingPairs: [
				{ open: "{", close: "}" },
				{ open: "[", close: "]" },
				{ open: "(", close: ")" },
				{ open: '"', close: '"' },
			],
			indentationRules: {
				increaseIndentPattern: /^.*\{[^}"']*$/,
				decreaseIndentPattern: /^\s*\}/,
			},
		});
		monaco.languages.setMonarchTokensProvider(FLOWSCRIPT_LANGUAGE_ID, {
			tokenizer: {
				root: [
					[/\/\/@[nvl]:.*$/, "comment.flow-anchor"],
					[/\/\/.*$/, "comment"],
					[/@"?[A-Za-z_$][\w$]*"?/, "annotation"],
					[/"([^"\\]|\\.)*$/, "string.invalid"],
					[/"/, "string", "@string"],
					[/\b(const|let|function)\b/, "keyword.storage"],
					[/\b(if|else|for|of|return)\b/, "keyword.control"],
					[
						/\b(string|int|float|bool|void|Date|Generic|Byte|PathBuf|Struct|Map|Set)\b/,
						"type",
					],
					[/\b(true|false|null)\b/, "constant.language"],
					[/\b-?\d+\.\d+([eE][+-]?\d+)?\b/, "number.float"],
					[/\b-?\d+\b/, "number"],
					[/[A-Za-z_$][\w$]*(?=\s*\()/, "entity.name.function"],
					[/[A-Za-z_$][\w$]*(?=\s*:)/, "variable.parameter"],
					[/[A-Za-z_$][\w$]*/, "identifier"],
					[/===|!==|==|!=|>=|<=|>|<|&&|\|\||!|\+|-|\*|\/|%|=|\?/, "operator"],
					[/[{}()[\]]/, "@brackets"],
				],
				string: [
					[/[^\\"]+/, "string"],
					[/\\(?:["\\/nrt]|u[0-9A-Fa-f]{4})/, "string.escape"],
					[/"/, "string", "@pop"],
				],
			},
		});
		flowScriptLanguageRegistered = true;
	} else if (hasRegisteredLanguage) {
		flowScriptLanguageRegistered = true;
	}

	monaco.editor.defineTheme("flowpilot-flowscript-light", {
		base: "vs",
		inherit: true,
		rules: [
			{ token: "comment", foreground: "7a7f8a", fontStyle: "italic" },
			{ token: "comment.flow-anchor", foreground: "2563eb", fontStyle: "bold" },
			{ token: "annotation", foreground: "8b5cf6" },
			{ token: "keyword", foreground: "b91c6b", fontStyle: "bold" },
			{ token: "keyword.storage", foreground: "315ac5", fontStyle: "bold" },
			{ token: "type", foreground: "6d55c7" },
			{ token: "entity.name.function", foreground: "087ea4" },
			{ token: "variable.parameter", foreground: "a56a00" },
			{ token: "string", foreground: "159447" },
			{ token: "number", foreground: "c2410c" },
			{ token: "constant.language", foreground: "7c3aed" },
			{ token: "operator", foreground: "5b6270" },
		],
		colors: {
			"editor.background": "#fbfafc",
			"editor.foreground": "#24252b",
			"editorGutter.background": "#fbfafc",
			"editorLineNumber.foreground": "#a6a8b3",
			"editorLineNumber.activeForeground": "#6b7280",
			"editorCursor.foreground": "#ec4899",
			"editor.selectionBackground": "#8b5cf626",
			"editor.inactiveSelectionBackground": "#8b5cf617",
			"editor.lineHighlightBackground": "#11182708",
			"editorIndentGuide.background1": "#11182712",
			"editorIndentGuide.activeBackground1": "#8b5cf64a",
			"editorBracketMatch.background": "#8b5cf61c",
			"editorBracketMatch.border": "#8b5cf670",
			"scrollbarSlider.background": "#71717a33",
			"scrollbarSlider.hoverBackground": "#71717a4d",
			"scrollbarSlider.activeBackground": "#71717a66",
		},
	});
	monaco.editor.defineTheme("flowpilot-flowscript-dark", {
		base: "vs-dark",
		inherit: true,
		rules: [
			{ token: "comment", foreground: "a1a1aa", fontStyle: "italic" },
			{ token: "comment.flow-anchor", foreground: "38bdf8", fontStyle: "bold" },
			{ token: "annotation", foreground: "c084fc" },
			{ token: "keyword", foreground: "f472b6", fontStyle: "bold" },
			{ token: "keyword.storage", foreground: "60a5fa", fontStyle: "bold" },
			{ token: "type", foreground: "a78bfa" },
			{ token: "entity.name.function", foreground: "22d3ee" },
			{ token: "variable.parameter", foreground: "facc15" },
			{ token: "string", foreground: "86efac" },
			{ token: "number", foreground: "fb923c" },
			{ token: "constant.language", foreground: "c084fc" },
			{ token: "operator", foreground: "d4d4d8" },
		],
		colors: {
			"editor.background": "#111116",
			"editor.foreground": "#e5e7eb",
			"editorGutter.background": "#111116",
			"editorLineNumber.foreground": "#686b76",
			"editorLineNumber.activeForeground": "#d4d4d8",
			"editorCursor.foreground": "#f472b6",
			"editor.selectionBackground": "#a855f733",
			"editor.inactiveSelectionBackground": "#a855f71f",
			"editor.lineHighlightBackground": "#ffffff08",
			"editorIndentGuide.background1": "#ffffff12",
			"editorIndentGuide.activeBackground1": "#a855f65c",
			"editorBracketMatch.background": "#a855f61f",
			"editorBracketMatch.border": "#c084fc70",
			"scrollbarSlider.background": "#a1a1aa33",
			"scrollbarSlider.hoverBackground": "#a1a1aa4d",
			"scrollbarSlider.activeBackground": "#a1a1aa66",
		},
	});
	monaco.editor.setTheme(
		isDark ? "flowpilot-flowscript-dark" : "flowpilot-flowscript-light",
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

function parseFlowScriptWorkspaceEvent(payload: string): {
	source: string;
	status?: string;
} | null {
	try {
		const parsed = JSON.parse(payload);
		if (typeof parsed === "string") return { source: parsed };
		if (parsed && typeof parsed.source === "string") {
			return {
				source: parsed.source,
				status: typeof parsed.status === "string" ? parsed.status : undefined,
			};
		}
	} catch {
		return null;
	}
	return null;
}

function parseStreamJson(payload: string): Record<string, unknown> | null {
	try {
		const parsed = JSON.parse(payload);
		return parsed && typeof parsed === "object" ? parsed : null;
	} catch {
		return null;
	}
}

function normalizeEnabledAIProvider(
	provider?: AIProvider,
): NormalizedAIProvider {
	const normalized = normalizeAIProvider(provider);
	return normalized === "claude-code" ? "codex" : normalized;
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
		case "edit_flowscript":
			return "Editing FlowScript";
		case "catalog_search":
			return "Searching catalog";
		case "validate_commands":
			return "Validating board changes";
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
		case "database_tool":
			return "Using database";
		case "storage_tool":
			return "Using storage";
		case "execute_event":
			return "Executing event";
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

function formatLineCount(source: string): string {
	const lines = source ? source.split("\n").length : 0;
	return `${lines} line${lines === 1 ? "" : "s"}`;
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

type TauriHttpModule = {
	fetch: typeof fetch;
};

const FLOWPILOT_FRONTEND_TOOL_EVENT = "flowpilot://frontend-tool-request";

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

async function importTauriHttp(): Promise<TauriHttpModule> {
	return import("@tauri-apps/plugin-http") as Promise<TauriHttpModule>;
}

function getArgString(
	args: Record<string, unknown>,
	snake: string,
	camel = snake,
): string | undefined {
	const value = args[snake] ?? args[camel];
	return typeof value === "string" && value.trim() ? value : undefined;
}

function getArgBool(
	args: Record<string, unknown>,
	snake: string,
	camel = snake,
	defaultValue = false,
): boolean {
	const value = args[snake] ?? args[camel];
	return typeof value === "boolean" ? value : defaultValue;
}

function getArgNumber(
	args: Record<string, unknown>,
	snake: string,
	camel = snake,
	defaultValue = 0,
): number {
	const value = args[snake] ?? args[camel];
	return typeof value === "number" && Number.isFinite(value)
		? value
		: defaultValue;
}

function clampToolLimit(value: number, defaultValue: number, maxValue: number) {
	if (!Number.isFinite(value) || value <= 0) return defaultValue;
	return Math.min(Math.floor(value), maxValue);
}

function resolveToolAppId(
	args: Record<string, unknown>,
	defaultAppId?: string,
): string {
	const appId = getArgString(args, "app_id", "appId") ?? defaultAppId;
	if (!appId) {
		throw new Error(
			"Missing app_id. Provide app_id or open FlowPilot from an app context.",
		);
	}
	return appId;
}

function mapIndexType(value: unknown): IIndexType {
	const normalized = String(value ?? "Auto")
		.replace(/[\s-]/g, "_")
		.toLowerCase();
	switch (normalized) {
		case "fulltext":
		case "full_text":
			return IIndexType.FullText;
		case "btree":
		case "b_tree":
			return IIndexType.BTree;
		case "bitmap":
			return IIndexType.Bitmap;
		case "labellist":
		case "label_list":
			return IIndexType.LabelList;
		default:
			return IIndexType.Auto;
	}
}

function splitStoragePath(path: string): { prefix: string; fileName: string } {
	const normalized = path.replace(/^\/+/, "");
	const lastSlash = normalized.lastIndexOf("/");
	if (lastSlash < 0) return { prefix: "", fileName: normalized };
	return {
		prefix: normalized.slice(0, lastSlash),
		fileName: normalized.slice(lastSlash + 1),
	};
}

function compactJson(value: unknown, maxChars = 12_000): unknown {
	try {
		const text = JSON.stringify(value);
		if (text.length <= maxChars) return value;
		return {
			truncated: true,
			chars: text.length,
			preview: text.slice(0, maxChars),
		};
	} catch {
		return String(value);
	}
}

function compactLogEvents(events: unknown[], maxEvents = 80): unknown[] {
	return events.slice(-maxEvents).map((event) => {
		if (!event || typeof event !== "object") return event;
		const object = event as Record<string, unknown>;
		return {
			event_type: object.event_type,
			payload: compactJson(object.payload, 3000),
		};
	});
}

async function fetchJsonViaTauri(url: string): Promise<unknown> {
	try {
		if (!isTauriRuntime()) throw new Error("Tauri runtime is unavailable.");
		const http = await importTauriHttp();
		const response = await http.fetch(url, { method: "GET" });
		return await response.json();
	} catch {
		const response = await fetch(url);
		if (!response.ok) {
			throw new Error(`Search request failed with HTTP ${response.status}`);
		}
		return await response.json();
	}
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

export function FlowPilot({
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
	const [elapsedSeconds, setElapsedSeconds] = useState(0);
	const [tokenCount, setTokenCount] = useState(0);
	const [planSteps, setPlanSteps] = useState<UnifiedPlanStep[]>([]);
	const [attachedImages, setAttachedImages] = useState<AttachedImage[]>([]);
	const [userScrolledUp, setUserScrolledUp] = useState(false);
	const [selectedModelId, setSelectedModelId] = useState("");

	// Provider state
	const [provider, setProvider] = useState<AIProvider>(
		normalizeEnabledAIProvider(forceProvider ?? defaultProvider),
	);
	const normalizedProvider = normalizeAIProvider(provider);
	const activeAgentBackend: AgentBackendProvider = isAgentBackendProvider(
		normalizedProvider,
	)
		? normalizedProvider
		: "github-copilot";

	// Board-specific state
	const [pendingCommands, setPendingCommands] = useState<BoardCommand[]>([]);
	const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
	const [currentToolCall, setCurrentToolCall] = useState<string | null>(null);
	const [flowscriptWorkspace, setFlowscriptWorkspace] = useState("");
	const [flowscriptWorkspaceStatus, setFlowscriptWorkspaceStatus] = useState<
		string | undefined
	>();
	const [appliedFlowScriptWorkspace, setAppliedFlowScriptWorkspace] =
		useState("");
	const [destructiveApplyRequest, setDestructiveApplyRequest] = useState<{
		flowscript: string;
		diagnostic: string;
	} | null>(null);
	const [destructiveApplyPending, setDestructiveApplyPending] = useState(false);
	const [showWorkspace, setShowWorkspace] = useState(false);
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

	// Refs
	const messagesEndRef = useRef<HTMLDivElement>(null);
	const scrollContainerRef = useRef<HTMLDivElement>(null);
	const imageInputRef = useRef<HTMLInputElement>(null);
	const initialPromptHandledRef = useRef(false);
	const handleSubmitRef = useRef<(() => void) | null>(null);

	// Backend context
	const backendContext = useBackend();
	const executionService = useExecutionServiceOptional();
	const activeAppId = appId ?? runContext?.app_id;
	const approvedFrontendToolKeysRef = useRef<Set<string>>(new Set());
	const frontendToolDialogResolverRef = useRef<((value: any) => void) | null>(
		null,
	);
	const frontendToolDialogQueueRef = useRef<FrontendToolQueuedDialog[]>([]);
	const [frontendToolDialog, setFrontendToolDialog] =
		useState<FrontendToolDialogState | null>(null);

	// Agent backend hook
	const copilotSDK = useCopilotSDK(activeAgentBackend);

	// Elapsed time tracking
	useEffect(() => {
		if (!loading || !loadingStartTime) {
			setElapsedSeconds(0);
			return;
		}
		const interval = setInterval(() => {
			setElapsedSeconds(Math.floor((Date.now() - loadingStartTime) / 1000));
		}, 1000);
		return () => clearInterval(interval);
	}, [loading, loadingStartTime]);

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

	// Filter bits models to only include those in the user's profile
	const bitsModels = useMemo(() => {
		if (!foundBits.data || !profile.data?.hub_profile.bits) return [];
		const profileBitIds = new Set(profile.data.hub_profile.bits);
		const canHostLocal = backendContext.capabilities().canHostLlamaCPP;

		return foundBits.data.filter((model) => {
			const fullId = `${model.hub}:${model.id}`;
			if (!profileBitIds.has(fullId)) return false;

			if (!canHostLocal) {
				const providerName =
					model.parameters?.provider?.provider_name?.toLowerCase();
				if (
					providerName === "local" ||
					providerName === "llama.cpp" ||
					providerName === "llamacpp" ||
					providerName === "ollama"
				) {
					return false;
				}
			}

			return true;
		});
	}, [foundBits.data, profile.data?.hub_profile.bits]);

	const openFrontendToolDialog = useCallback(
		(dialog: FrontendToolDialogState, resolve: (value: any) => void) => {
			if (frontendToolDialogResolverRef.current) {
				frontendToolDialogQueueRef.current.push({ dialog, resolve });
				return;
			}
			frontendToolDialogResolverRef.current = resolve;
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
			if (
				approval?.kind === "none" ||
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
		[openFrontendToolDialog],
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

	const runInternetSearchTool = useCallback(
		async (args: Record<string, unknown>) => {
			const query = getArgString(args, "query");
			if (!query) throw new Error("internet_search requires query.");
			const language = getArgString(args, "language") ?? "en-US";
			const page = Math.max(1, getArgNumber(args, "page", "page", 1));
			const limit = clampToolLimit(
				getArgNumber(args, "limit", "limit", 8),
				8,
				20,
			);
			const url = new URL("https://search.flow-like.com/search");
			url.searchParams.set("q", query);
			url.searchParams.set("format", "json");
			url.searchParams.set("pageno", String(page));
			url.searchParams.set("language", language);

			const json = (await fetchJsonViaTauri(url.toString())) as Record<
				string,
				unknown
			>;
			const results = Array.isArray(json.results) ? json.results : [];
			return {
				status: "ok",
				query,
				page,
				results: results.slice(0, limit).map((result) => {
					const item = result as Record<string, unknown>;
					return {
						title: item.title,
						url: item.url,
						content: item.content,
						publishedDate: item.publishedDate,
						engine: item.engine,
						category: item.category,
						score: item.score,
					};
				}),
			};
		},
		[],
	);

	const runDatabaseTool = useCallback(
		async (args: Record<string, unknown>) => {
			const operation = getArgString(args, "operation") ?? "list_tables";
			const toolAppId = resolveToolAppId(args, activeAppId);
			const tableName = getArgString(args, "table_name", "tableName");
			const userScoped = getArgBool(args, "user_scoped", "userScoped", false);
			const offset = Math.max(0, getArgNumber(args, "offset", "offset", 0));
			const limit = clampToolLimit(
				getArgNumber(
					args,
					"limit",
					"limit",
					operation === "describe_table" ? 10 : 50,
				),
				operation === "describe_table" ? 10 : 50,
				200,
			);

			switch (operation) {
				case "list_tables": {
					const [projectTables, userTables] = await Promise.all([
						backendContext.dbState.listTables(toolAppId),
						backendContext.dbState.listTablesUser(toolAppId),
					]);
					return {
						status: "ok",
						app_id: toolAppId,
						project_tables: projectTables,
						user_tables: userTables,
					};
				}
				case "describe_table": {
					if (!tableName)
						throw new Error("describe_table requires table_name.");
					const [schema, indices, rowCount, sample] = await Promise.all([
						backendContext.dbState.getSchema(toolAppId, tableName, userScoped),
						backendContext.dbState.getIndices(toolAppId, tableName, userScoped),
						backendContext.dbState.countItems(toolAppId, tableName, userScoped),
						backendContext.dbState.listItems(
							toolAppId,
							tableName,
							0,
							limit,
							userScoped,
						),
					]);
					return {
						status: "ok",
						table_name: tableName,
						user_scoped: userScoped,
						schema,
						indices,
						row_count: rowCount,
						sample,
					};
				}
				case "query": {
					if (!tableName) throw new Error("query requires table_name.");
					const query =
						args.query && typeof args.query === "object"
							? (args.query as Record<string, unknown>)
							: {};
					const rows = await backendContext.dbState.queryItems(
						toolAppId,
						tableName,
						query,
						offset,
						limit,
						userScoped,
					);
					return {
						status: "ok",
						table_name: tableName,
						user_scoped: userScoped,
						row_count: rows.length,
						rows,
					};
				}
				case "insert":
				case "add_items": {
					if (!tableName) throw new Error("insert requires table_name.");
					const items = Array.isArray(args.items) ? args.items : [];
					if (items.length === 0)
						throw new Error("insert requires non-empty items.");
					await backendContext.dbState.addItems(
						toolAppId,
						tableName,
						items,
						userScoped,
					);
					return {
						status: "ok",
						inserted: items.length,
						table_name: tableName,
					};
				}
				case "delete":
				case "remove_items": {
					if (!tableName) throw new Error("delete requires table_name.");
					const filter = getArgString(args, "filter");
					if (!filter) throw new Error("delete requires filter.");
					await backendContext.dbState.removeItems(
						toolAppId,
						tableName,
						filter,
						userScoped,
					);
					return { status: "ok", table_name: tableName, filter };
				}
				case "update": {
					if (!tableName) throw new Error("update requires table_name.");
					const filter = getArgString(args, "filter");
					const updates =
						args.updates && typeof args.updates === "object"
							? (args.updates as Record<string, unknown>)
							: undefined;
					if (!filter || !updates) {
						throw new Error("update requires filter and updates.");
					}
					await backendContext.dbState.updateItem(
						toolAppId,
						tableName,
						filter,
						updates,
						userScoped,
					);
					return { status: "ok", table_name: tableName, filter };
				}
				case "build_index": {
					if (!tableName) throw new Error("build_index requires table_name.");
					const column = getArgString(args, "column");
					if (!column) throw new Error("build_index requires column.");
					await backendContext.dbState.buildIndex(
						toolAppId,
						tableName,
						column,
						mapIndexType(args.index_type ?? args.indexType),
						getArgBool(args, "optimize", "optimize", false),
						userScoped,
					);
					return { status: "ok", table_name: tableName, column };
				}
				case "drop_index": {
					if (!tableName) throw new Error("drop_index requires table_name.");
					const indexName = getArgString(args, "index_name", "indexName");
					if (!indexName) throw new Error("drop_index requires index_name.");
					await backendContext.dbState.dropIndex(
						toolAppId,
						tableName,
						indexName,
						userScoped,
					);
					return { status: "ok", table_name: tableName, index_name: indexName };
				}
				case "optimize": {
					if (!tableName) throw new Error("optimize requires table_name.");
					await backendContext.dbState.optimize(
						toolAppId,
						tableName,
						getArgBool(args, "keep_versions", "keepVersions", false),
						userScoped,
					);
					return { status: "ok", table_name: tableName };
				}
				case "add_column": {
					if (!tableName) throw new Error("add_column requires table_name.");
					const column =
						args.column_definition && typeof args.column_definition === "object"
							? (args.column_definition as {
									name: string;
									sql_expression: string;
								})
							: undefined;
					if (!column?.name || !column?.sql_expression) {
						throw new Error(
							"add_column requires column_definition.name and sql_expression.",
						);
					}
					await backendContext.dbState.addColumn(
						toolAppId,
						tableName,
						column,
						userScoped,
					);
					return { status: "ok", table_name: tableName, column: column.name };
				}
				case "drop_columns": {
					if (!tableName) throw new Error("drop_columns requires table_name.");
					const columns = Array.isArray(args.columns)
						? args.columns.filter(
								(value): value is string => typeof value === "string",
							)
						: [];
					if (columns.length === 0)
						throw new Error("drop_columns requires columns.");
					await backendContext.dbState.dropColumns(
						toolAppId,
						tableName,
						columns,
						userScoped,
					);
					return { status: "ok", table_name: tableName, columns };
				}
				case "alter_column": {
					if (!tableName) throw new Error("alter_column requires table_name.");
					const column = getArgString(args, "column");
					if (!column) throw new Error("alter_column requires column.");
					await backendContext.dbState.alterColumn(
						toolAppId,
						tableName,
						column,
						getArgBool(args, "nullable", "nullable", true),
						userScoped,
					);
					return { status: "ok", table_name: tableName, column };
				}
				default:
					throw new Error(
						`Unsupported database_tool operation '${operation}'.`,
					);
			}
		},
		[activeAppId, backendContext.dbState],
	);

	const runStorageTool = useCallback(
		async (args: Record<string, unknown>) => {
			const operation = getArgString(args, "operation") ?? "list_files";
			const toolAppId = resolveToolAppId(args, activeAppId);
			const userScoped = getArgBool(args, "user_scoped", "userScoped", false);
			const storage = backendContext.storageState;
			const list = userScoped
				? storage.listStorageItemsUser.bind(storage)
				: storage.listStorageItems.bind(storage);
			const download = userScoped
				? storage.downloadStorageItemsUser.bind(storage)
				: storage.downloadStorageItems.bind(storage);
			const upload = userScoped
				? storage.uploadStorageItemsUser.bind(storage)
				: storage.uploadStorageItems.bind(storage);
			const remove = userScoped
				? storage.deleteStorageItemsUser.bind(storage)
				: storage.deleteStorageItems.bind(storage);

			switch (operation) {
				case "list_files": {
					const prefix = getArgString(args, "prefix") ?? "";
					const items = await list(toolAppId, prefix);
					return {
						status: "ok",
						prefix,
						user_scoped: userScoped,
						items,
					};
				}
				case "read_file": {
					const path = getArgString(args, "path");
					if (!path) throw new Error("read_file requires path.");
					const maxChars = clampToolLimit(
						getArgNumber(args, "max_chars", "maxChars", 20_000),
						20_000,
						120_000,
					);
					const [file] = await download(toolAppId, [path]);
					if (!file || file.error) {
						throw new Error(
							file?.error ?? `Unable to resolve storage path '${path}'.`,
						);
					}
					if (!file.url) {
						return {
							status: "ok",
							path,
							message: "Storage provider returned no readable URL.",
						};
					}
					const response = await fetch(file.url);
					const content = await response.text();
					return {
						status: "ok",
						path,
						url: file.url,
						truncated: content.length > maxChars,
						content: content.slice(0, maxChars),
						chars: content.length,
					};
				}
				case "create_file": {
					const path = getArgString(args, "path");
					if (!path) throw new Error("create_file requires path.");
					const content = String(args.content ?? "");
					const mimeType =
						getArgString(args, "mime_type", "mimeType") ?? "text/plain";
					const { prefix, fileName } = splitStoragePath(path);
					if (!fileName)
						throw new Error("create_file path must include a file name.");
					const file = new File([content], fileName, { type: mimeType });
					await upload(toolAppId, prefix, [file]);
					return {
						status: "ok",
						path,
						bytes: new Blob([content]).size,
						user_scoped: userScoped,
					};
				}
				case "delete_files": {
					const paths = Array.isArray(args.paths)
						? args.paths.filter(
								(value): value is string => typeof value === "string",
							)
						: getArgString(args, "path")
							? [getArgString(args, "path") as string]
							: [];
					if (paths.length === 0)
						throw new Error("delete_files requires paths.");
					await remove(toolAppId, paths);
					return { status: "ok", deleted: paths, user_scoped: userScoped };
				}
				default:
					throw new Error(`Unsupported storage_tool operation '${operation}'.`);
			}
		},
		[activeAppId, backendContext.storageState],
	);

	const runExecuteEventTool = useCallback(
		async (args: Record<string, unknown>) => {
			const toolAppId = resolveToolAppId(args, activeAppId);
			const eventId = getArgString(args, "event_id", "eventId");
			if (!eventId) throw new Error("execute_event requires event_id.");
			const streamState = getArgBool(args, "stream_state", "streamState", true);
			const skipConsentCheck = getArgBool(
				args,
				"skip_consent_check",
				"skipConsentCheck",
				false,
			);
			const payload =
				args.payload && typeof args.payload === "object"
					? ({
							id: eventId,
							...(args.payload as Record<string, unknown>),
						} as any)
					: { id: eventId, payload: {} };
			const logs: unknown[] = [];
			let runId: string | undefined;
			const execute =
				executionService?.executeEvent ??
				backendContext.eventState.executeEvent.bind(backendContext.eventState);
			const metadata = await execute(
				toolAppId,
				eventId,
				payload,
				streamState,
				(id) => {
					runId = id;
				},
				(events) => {
					logs.push(...events);
				},
				skipConsentCheck,
			);
			return {
				status: "ok",
				app_id: toolAppId,
				event_id: eventId,
				run_id: runId,
				metadata,
				log_count: logs.length,
				logs: compactLogEvents(logs),
			};
		},
		[activeAppId, backendContext.eventState, executionService],
	);

	const executeFrontendToolRequest = useCallback(
		async (request: FrontendToolRequest): Promise<FrontendToolResponse> => {
			try {
				if (request.toolName === "ask_user") {
					const answer = await requestFrontendUserInput(request);
					return {
						requestId: request.requestId,
						approved: true,
						result: { status: "ok", answer },
					};
				}

				const approval = await requestFrontendToolApproval(request);
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
					approvedFrontendToolKeysRef.current.add(sessionKey);
				}

				let result: unknown;
				switch (request.toolName) {
					case "internet_search":
						result = await runInternetSearchTool(request.arguments);
						break;
					case "database_tool":
						result = await runDatabaseTool(request.arguments);
						break;
					case "storage_tool":
						result = await runStorageTool(request.arguments);
						break;
					case "execute_event":
						result = await runExecuteEventTool(request.arguments);
						break;
					default:
						throw new Error(`Unsupported frontend tool '${request.toolName}'.`);
				}

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
		[
			requestFrontendToolApproval,
			requestFrontendUserInput,
			runDatabaseTool,
			runExecuteEventTool,
			runInternetSearchTool,
			runStorageTool,
		],
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

				const stop = await eventApi.listen<FrontendToolRequest>(
					FLOWPILOT_FRONTEND_TOOL_EVENT,
					async (event) => {
						if (disposed) return;
						const request = event.payload;
						if (!request?.requestId || !request.toolName) return;
						const response =
							await executeFrontendToolRequestRef.current(request);
						if (disposed) return;
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
					},
				);

				if (disposed) {
					stop();
				} else {
					unlisten = stop;
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
		};
	}, []);

	const resolveFrontendToolDialog = useCallback((value: unknown) => {
		const resolver = frontendToolDialogResolverRef.current;
		frontendToolDialogResolverRef.current = null;
		resolver?.(value);

		const next = frontendToolDialogQueueRef.current.shift();
		if (next) {
			frontendToolDialogResolverRef.current = next.resolve;
			setFrontendToolDialog(next.dialog);
		} else {
			setFrontendToolDialog(null);
		}
	}, []);

	// Get current models based on provider
	const currentModels = useMemo(() => {
		if (isAgentBackendProvider(normalizedProvider)) {
			return copilotSDK.models;
		}
		return bitsModels;
	}, [normalizedProvider, copilotSDK.models, bitsModels]);
	const previousModelProviderRef = useRef(normalizedProvider);

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
				preferredModel =
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
		await copilotSDK.stop();
		setProvider("bits");
	}, [copilotSDK]);

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
	const handleNewChat = useCallback(() => {
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
		setAppliedFlowScriptWorkspace("");
		setDestructiveApplyRequest(null);
		setDestructiveApplyPending(false);
		setShowWorkspace(false);
		setProcessEvents([]);
		setCurrentConversationId(undefined);
		currentMessageIdRef.current = undefined;
		setShowHistory(false);
	}, []);

	// Select conversation from history
	const handleSelectConversation = useCallback(
		async (conversation: IFlowPilotConversation) => {
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
				}));
				const latestWorkspace = [...loadedMessages]
					.reverse()
					.find((message) => message.flowscriptWorkspace)?.flowscriptWorkspace;
				setMessages(loadedMessages);
				setFlowscriptWorkspace(latestWorkspace ?? "");
				setFlowscriptWorkspaceStatus(undefined);
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
				setShowHistory(false);
			} catch (err) {
				console.error("Failed to load conversation:", err);
			}
		},
		[],
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
				.find((message) => message.role === "assistant");
			const nextExecutedCommands = [
				...(lastAssistantMessage?.executedCommands ?? []),
				...appliedBoardCommands,
			];
			setMessages((prev) => {
				const newMessages = [...prev];
				for (let i = newMessages.length - 1; i >= 0; i--) {
					if (newMessages[i].role === "assistant") {
						const existingCommands = newMessages[i].executedCommands || [];
						newMessages[i] = {
							...newMessages[i],
							executedCommands: [
								...existingCommands,
								...appliedBoardCommands,
							],
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

	// Board mode handlers
	const handleExecuteCommands = useCallback(async () => {
		const shouldApplyFlowScript =
			Boolean(onApplyFlowScript) &&
			flowscriptWorkspace.trim().length > 0 &&
			flowscriptWorkspaceStatus !== "validation_errors" &&
			flowscriptWorkspaceStatus !== "no_changes" &&
			flowscriptWorkspace !== appliedFlowScriptWorkspace;
		if (
			shouldApplyFlowScript ||
			(onExecuteCommands && pendingCommands.length > 0)
		) {
			let appliedBoardCommands: BoardCommand[] = pendingCommands;
			try {
				let applyResult: unknown;
				if (shouldApplyFlowScript && onApplyFlowScript) {
					applyResult = await onApplyFlowScript(flowscriptWorkspace, {
						suppressBlockedToast: true,
					});
					if (!applyResult) return;

					appliedBoardCommands = applyResultBoardCommands(applyResult);
				} else if (onExecuteCommands) {
					await onExecuteCommands(pendingCommands);
				}
				const diagnostics = applyResultDiagnostics(applyResult);
				if (applyResultCommandCount(applyResult) === 0 && diagnostics.length > 0) {
					const destructiveDiagnostic =
						destructiveFlowScriptDiagnostic(diagnostics);
					if (shouldApplyFlowScript && destructiveDiagnostic) {
						setDestructiveApplyRequest({
							flowscript: flowscriptWorkspace,
							diagnostic: destructiveDiagnostic,
						});
						return;
					}
					setFlowscriptWorkspaceStatus("validation_errors");
					return;
				}
				if (shouldApplyFlowScript) {
					setAppliedFlowScriptWorkspace(flowscriptWorkspace);
					setFlowscriptWorkspaceStatus("applied");
				}
			} catch (error) {
				console.error("Failed to apply FlowPilot commands:", error);
				return;
			}
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
		recordExecutedBoardCommands,
	]);

	const handleExecuteSingle = useCallback(
		async (index: number) => {
			if (onExecuteCommands && pendingCommands[index]) {
				const command = pendingCommands[index];
				const lastAssistantMessage = [...messages]
					.reverse()
					.find((message) => message.role === "assistant");
				const nextExecutedCommands = [
					...(lastAssistantMessage?.executedCommands ?? []),
					command,
				];
				try {
					await onExecuteCommands([command]);
				} catch (error) {
					console.error("Failed to apply FlowPilot command:", error);
					return;
				}
				setMessages((prev) => {
					const newMessages = [...prev];
					for (let i = newMessages.length - 1; i >= 0; i--) {
						if (newMessages[i].role === "assistant") {
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
		[messages, onExecuteCommands, pendingCommands],
	);

	const handleDismissCommands = useCallback(() => {
		if (flowscriptWorkspace) {
			setAppliedFlowScriptWorkspace(flowscriptWorkspace);
			setFlowscriptWorkspaceStatus("dismissed");
		}
		setPendingCommands([]);
		setDestructiveApplyRequest(null);
	}, [flowscriptWorkspace]);

	const handleApproveFlowScriptDeletion = useCallback(async () => {
		if (!destructiveApplyRequest || !onApplyFlowScript) return;

		setDestructiveApplyPending(true);
		try {
			const applyResult = await onApplyFlowScript(
				destructiveApplyRequest.flowscript,
				{ allowDeletions: true },
			);
			if (!applyResult) return;

			const diagnostics = applyResultDiagnostics(applyResult);
			if (applyResultCommandCount(applyResult) === 0 && diagnostics.length > 0) {
				setFlowscriptWorkspaceStatus("validation_errors");
				setDestructiveApplyRequest(null);
				return;
			}

			recordExecutedBoardCommands(applyResultBoardCommands(applyResult));
			setAppliedFlowScriptWorkspace(destructiveApplyRequest.flowscript);
			setFlowscriptWorkspaceStatus("applied");
			setPendingCommands([]);
			setDestructiveApplyRequest(null);
		} catch (error) {
			console.error("Failed to apply destructive FlowScript edit:", error);
		} finally {
			setDestructiveApplyPending(false);
		}
	}, [
		destructiveApplyRequest,
		onApplyFlowScript,
		recordExecutedBoardCommands,
	]);

	// UI mode handlers
	const handleApplyComponents = useCallback(() => {
		if (pendingComponents.length > 0) {
			const nextAppliedComponents = [...pendingComponents];
			onApplyComponents?.(pendingComponents, pendingCanvasSettings);
			setMessages((prev) => {
				const newMessages = [...prev];
				for (let i = newMessages.length - 1; i >= 0; i--) {
					if (newMessages[i].role === "assistant") {
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

			if (
				isAgentBackendProvider(normalizedProvider) &&
				(!copilotSDK.isRunning || !selectedModelId)
			) {
				setMessages((prev) => [
					...prev,
					{
						role: "assistant",
						content: copilotSDK.error
							? `Agent backend is not ready yet: ${copilotSDK.error}`
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
			setTokenCount(0);
			setPlanSteps([]);
			setProcessEvents([]);
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
			try {
				let currentMessageContent = "";
				let lastUpdateTime = 0;
				const UPDATE_INTERVAL = 100;
				let tagBuffer = ""; // Buffer for partial XML tags that might be split across tokens
				let currentPlanSteps: UnifiedPlanStep[] = [];
				let latestFlowScriptWorkspace = flowscriptWorkspace;
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

				const applyFlowScriptWorkspace = (workspace: string, status?: string) => {
					const source = workspace;
					if (!source.trim()) return;
					const previousWorkspace = latestFlowScriptWorkspace;
					latestFlowScriptWorkspace = source;
					setFlowscriptWorkspace(source);
					setFlowscriptWorkspaceStatus((previousStatus) =>
						status ?? (previousWorkspace === source ? previousStatus : "queued"),
					);
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

				const onToken = (rawToken: string) => {
					setTokenCount((prev) => prev + 1);

					// Combine with buffer for partial tags
					let token = tagBuffer + rawToken;
					tagBuffer = "";

					// Check if we have an incomplete XML tag at the end
					const lastOpenTag = token.lastIndexOf("<");
					if (lastOpenTag !== -1 && !token.slice(lastOpenTag).includes(">")) {
						// Incomplete tag - buffer it for next token
						tagBuffer = token.slice(lastOpenTag);
						token = token.slice(0, lastOpenTag);
						if (!token) return; // Nothing to process yet
					}

					// Parse scope decision events (skip them - they're internal)
					const scopeDecisionMatch = token.match(
						/<scope_decision>([\s\S]*?)<\/scope_decision>/,
					);
					if (scopeDecisionMatch) {
						return;
					}

					// Parse FlowScript workspace updates
					const workspaceMatch = token.match(
						/<flowscript_workspace>([\s\S]*?)<\/flowscript_workspace>/,
					);
					if (workspaceMatch) {
						const workspaceEvent = parseFlowScriptWorkspaceEvent(
							workspaceMatch[1],
						);
						if (workspaceEvent) {
							applyFlowScriptWorkspace(
								workspaceEvent.source,
								workspaceEvent.status,
							);
						}
						token = token.replace(
							/<flowscript_workspace>[\s\S]*?<\/flowscript_workspace>/g,
							"",
						);
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
								toolName === "internet_search"
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
								toolName === "edit_flowscript" ||
								toolName === "execute_event"
							) {
								setLoadingPhase("generating");
							} else if (toolName === "get_unconfigured_nodes") {
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
									details: event.details
										? `${event.details}\n\n${message}`
										: message,
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

					// Handle tool calls (board mode)
					if (token.includes("tool_call:")) {
						const match = token.match(/tool_call:(\w+)/);
						if (match) {
							const toolName = match[1];
							setCurrentToolCall(toolName);
							if (
								toolName.includes("search") ||
								toolName.includes("catalog") ||
								toolName.includes("filter")
							) {
								setLoadingPhase("searching");
							} else if (toolName === "think") {
								setLoadingPhase("reasoning");
							} else if (
								toolName === "emit_commands" ||
								toolName === "emit_surface" ||
								toolName === "edit_flowscript"
							) {
								setLoadingPhase("generating");
							}
						}
						return;
					}
					if (token.includes("tool_result:")) {
						setCurrentToolCall(null);
						return;
					}

					// Parse command blocks from Copilot SDK emit_commands tool
					const commandsMatch = token.match(/<commands>([\s\S]*?)<\/commands>/);
					if (commandsMatch) {
						try {
							const commands = JSON.parse(commandsMatch[1]);
							if (Array.isArray(commands) && commands.length > 0) {
								const flowScriptOwnsApply =
									latestFlowScriptWorkspace.trim().length > 0;
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

				// Build the prompt with context
				let userMsg = currentInput;
				if (runContext) {
					const runInfo = {
						run_id: runContext.run_id,
						app_id: runContext.app_id,
						board_id: runContext.board_id,
						event_id: runContext.event_id,
					};
					userMsg = `[RUN CONTEXT - User is asking about a flow execution run. Use the query_logs tool to fetch relevant logs.]
\`\`\`json
${JSON.stringify(runInfo, null, 2)}
\`\`\`

${currentInput}`;
				}

				if (scope === "Both") {
					userMsg = `[UNIFIED MODE - You can generate both workflow nodes AND UI components. If the user wants a UI, you can create A2UI components. If they want workflow automation, create nodes. You can also connect UI actions to workflows via action invokes.]

${userMsg}`;
				}

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

				const response = await backendContext.boardState.copilot_chat(
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
					undefined,
					backendRunContext,
					undefined, // actionContext - can be added later
				);

				flushMessageContent();

				const finalAssistantContent =
					currentMessageContent || response.message || "";
				if (response.flowscript_workspace) {
					applyFlowScriptWorkspace(response.flowscript_workspace);
				}

				// Save final assistant message to DB
				if (assistantMessageId && finalAssistantContent) {
					try {
						await updateMessage(assistantMessageId, {
							content: finalAssistantContent,
							flowscriptWorkspace: latestFlowScriptWorkspace || undefined,
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
						if (latestFlowScriptWorkspace) {
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
					latestFlowScriptWorkspace.trim().length === 0
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

				setLoadingPhase("finalizing");
			} catch (error) {
				console.error("FlowPilot error:", error);
				setMessages((prev) => {
					const newMessages = [...prev];
					const lastMessage = newMessages[newMessages.length - 1];
					if (lastMessage?.role === "assistant") {
						let errorMessage =
							error instanceof Error ? error.message : "Unknown error";

						if (
							errorMessage.includes("401 Unauthorized") ||
							errorMessage.includes("status code 401")
						) {
							errorMessage =
								"Authentication failed. Please check if you are signed in and your session is active.";
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
				if (phaseTimer) clearTimeout(phaseTimer);
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
			runContext,
			currentComponents,
			selectedComponentIds,
			onComponentsGenerated,
			backendContext.boardState,
			captureScreenshot,
			provider,
			normalizedProvider,
			copilotSDK.isRunning,
			currentConversationId,
			flowscriptWorkspace,
			loading,
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
	const flowscriptWorkspaceBlocksApply =
		flowscriptWorkspaceStatus === "validation_errors" ||
		flowscriptWorkspaceStatus === "no_changes";
	const hasUnappliedFlowScriptWorkspace =
		hasFlowScriptWorkspace &&
		Boolean(onApplyFlowScript) &&
		!flowscriptWorkspaceBlocksApply &&
		flowscriptWorkspace !== appliedFlowScriptWorkspace;
	const showFlowScriptWorkspace = hasFlowScriptWorkspace && showWorkspace;
	const visiblePendingCommands = hasUnappliedFlowScriptWorkspace
		? []
		: pendingCommands;

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
				elapsedSeconds={elapsedSeconds}
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
				hasWorkspace={hasFlowScriptWorkspace}
				showWorkspace={showWorkspace}
				onToggleWorkspace={() => setShowWorkspace((value) => !value)}
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
									const renderedMessage =
										isLastMessage &&
										processEvents.length > 0 &&
										(!message.processEvents ||
											message.processEvents.length < processEvents.length)
											? { ...message, processEvents }
											: message;
									return (
										<MessageBubble
											key={index}
											message={renderedMessage}
											isLoading={loading && isLastMessage}
											loadingPhase={loadingPhase}
											currentToolCall={currentToolCall}
											currentStep={
												loading && isLastMessage
													? planSteps.find((s) => s.status === "InProgress")
													: undefined
											}
											agentMode={agentMode}
											board={board}
											onFocusNode={onFocusNode}
											onSelectNodes={onSelectNodes}
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
					{(agentMode === "board" || agentMode === "both") &&
						(visiblePendingCommands.length > 0 ||
							hasUnappliedFlowScriptWorkspace) && (
							<div className="px-3 pb-2">
								<PendingCommandsView
									commands={visiblePendingCommands}
									flowscriptReady={hasUnappliedFlowScriptWorkspace}
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
					<FlowScriptWorkspacePanel source={flowscriptWorkspace} />
				)}
			</div>

			<FrontendToolRequestDialog
				dialog={frontendToolDialog}
				onDialogChange={setFrontendToolDialog}
				onResolve={resolveFrontendToolDialog}
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
							FlowScript apply needs to delete existing board items before it can
							continue.
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

interface FlowScriptWorkspacePanelProps {
	source: string;
}

const FlowScriptWorkspacePanel = memo(function FlowScriptWorkspacePanel({
	source,
}: FlowScriptWorkspacePanelProps) {
	const [copied, setCopied] = useState(false);
	const { resolvedTheme } = useTheme();
	const monacoRef = useRef<Monaco | null>(null);
	const isDark = resolvedTheme === "dark";

	const handleCopyWorkspace = useCallback(async () => {
		await navigator.clipboard.writeText(source);
		setCopied(true);
		setTimeout(() => setCopied(false), 1200);
	}, [source]);

	const handleBeforeMount = useCallback(
		(monaco: Monaco) => {
			monacoRef.current = monaco;
			registerFlowScriptLanguage(monaco, isDark);
		},
		[isDark],
	);

	useEffect(() => {
		if (!monacoRef.current) return;
		registerFlowScriptLanguage(monacoRef.current, isDark);
	}, [isDark]);

	return (
		<aside className="flex h-[42dvh] min-h-[260px] w-full shrink-0 flex-col border-t border-border/30 bg-muted/20 md:h-full md:min-h-0 md:w-[48%] md:min-w-[420px] md:max-w-[660px] md:border-l md:border-t-0">
			<div className="flex min-w-0 shrink-0 items-center justify-between gap-2 border-b border-border/30 bg-background/75 px-3 py-2.5 backdrop-blur-sm">
				<div className="flex min-w-0 items-center gap-2.5">
					<FileCode2Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
					<div className="min-w-0 text-sm font-semibold text-foreground">
						FlowScript
					</div>
					<div className="hidden truncate text-xs text-muted-foreground sm:block">
						Virtual workspace
					</div>
					<div className="rounded-full border border-border/50 bg-muted/40 px-2 py-0.5 font-mono text-[10px] text-muted-foreground">
						{formatLineCount(source)}
					</div>
				</div>
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="h-7 w-7 shrink-0 rounded-md"
							onClick={handleCopyWorkspace}
						>
							{copied ? (
								<CheckCircle2 className="h-4 w-4 text-green-600" />
							) : (
								<CopyIcon className="h-4 w-4" />
							)}
						</Button>
					</TooltipTrigger>
					<TooltipContent side="bottom" className="text-xs">
						Copy FlowScript
					</TooltipContent>
				</Tooltip>
			</div>
			<div className="min-h-0 flex-1 bg-linear-to-b from-muted/20 to-background/50 p-3">
				<div className="h-full min-h-0 overflow-hidden rounded-xl border border-border/45 bg-card shadow-[0_18px_45px_rgba(15,23,42,0.08)] ring-1 ring-black/[0.02] dark:shadow-black/25 dark:ring-white/[0.03]">
					<Editor
						beforeMount={handleBeforeMount}
						height="100%"
						language={FLOWSCRIPT_LANGUAGE_ID}
						theme={
							isDark
								? "flowpilot-flowscript-dark"
								: "flowpilot-flowscript-light"
						}
						value={source}
						options={{
							readOnly: true,
							automaticLayout: true,
							bracketPairColorization: { enabled: true },
							contextmenu: true,
							copyWithSyntaxHighlighting: true,
							cursorBlinking: "smooth",
							cursorSmoothCaretAnimation: "on",
							detectIndentation: false,
							fixedOverflowWidgets: true,
							folding: true,
							fontFamily:
								"JetBrains Mono, SFMono-Regular, ui-monospace, Menlo, Monaco, Consolas, monospace",
							fontLigatures: true,
							fontSize: 11,
							glyphMargin: false,
							guides: {
								bracketPairs: true,
								indentation: true,
							},
							lineDecorationsWidth: 6,
							lineHeight: 19,
							lineNumbers: "on",
							lineNumbersMinChars: 2,
							minimap: { enabled: false },
							overviewRulerBorder: false,
							overviewRulerLanes: 0,
							padding: { top: 14, bottom: 14 },
							renderLineHighlight: "line",
							renderWhitespace: "selection",
							scrollBeyondLastLine: false,
							scrollbar: {
								alwaysConsumeMouseWheel: false,
								horizontal: "auto",
								horizontalScrollbarSize: 8,
								useShadows: false,
								vertical: "auto",
								verticalScrollbarSize: 8,
							},
							smoothScrolling: true,
							stickyScroll: { enabled: false },
							tabSize: 2,
							wordWrap: "off",
							wrappingIndent: "indent",
						}}
					/>
				</div>
			</div>
		</aside>
	);
});

// Header component
interface HeaderProps {
	title: string;
	loading: boolean;
	loadingPhase: LoadingPhase;
	elapsedSeconds: number;
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
	hasWorkspace: boolean;
	showWorkspace: boolean;
	onToggleWorkspace: () => void;
}

const Header = memo(function Header({
	title,
	loading,
	loadingPhase,
	elapsedSeconds,
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
	hasWorkspace,
	showWorkspace,
	onToggleWorkspace,
}: HeaderProps) {
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
							<StatusPill
								phase={loadingPhase}
								elapsed={elapsedSeconds}
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

			{/* Provider and Model selector */}
			<div className="relative flex flex-col gap-2 px-3 pb-3 md:flex-row md:items-center">
				{/* Provider selector (only show if not forced) */}
				{!forceProvider && (
					<ProviderSelector
						provider={provider}
						onProviderChange={onProviderChange}
						copilotModels={copilotSDK.models}
						copilotAuthStatus={copilotSDK.authStatus}
						copilotRunning={copilotSDK.isRunning}
						copilotConnecting={copilotSDK.isConnecting}
						onStartCopilot={onStartCopilot}
						onStopCopilot={onStopCopilot}
						disabled={loading}
						className="w-full md:w-auto md:max-w-[60%]"
					/>
				)}

				{/* Model selector */}
				<ModelSelector
					provider={provider}
					bitsModels={bitsModels}
					copilotModels={copilotSDK.models}
					selectedModelId={selectedModelId}
					onModelChange={setSelectedModelId}
					disabled={loading}
					className="w-full min-w-0 md:flex-1"
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
}: MessageBubbleProps) {
	const isUser = message.role === "user";
	const hasProcessEvents =
		!isUser && message.processEvents && message.processEvents.length > 0;

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
											currentStep.tool_name === "edit_flowscript"
										? "Building flow"
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
					hasProcessEvents ? "w-full max-w-full" : "max-w-[85%]",
					isUser
						? "bg-muted/60 text-foreground rounded-br-sm border border-border/40"
						: "bg-background border border-border/40 rounded-bl-sm",
				)}
				style={{
					wordBreak: "break-word",
					overflowWrap: "anywhere",
					contain: hasProcessEvents ? "inline-size" : undefined,
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

				{/* Content */}
				{message.content ? (
					<MessageContent
						content={message.content}
						onFocusNode={onFocusNode}
						board={
							agentMode === "board" || agentMode === "both" ? board : undefined
						}
						enableMarkdown={true}
					/>
				) : isLoading ? null : (
					<p className="text-muted-foreground italic text-xs">No response</p>
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
