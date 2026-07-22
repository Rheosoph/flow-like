"use client";

import Editor, { type Monaco, type OnMount } from "@monaco-editor/react";
import {
	AlertTriangleIcon,
	CopyIcon,
	FileCode2Icon,
	Loader2Icon,
	RefreshCcwIcon,
	Undo2Icon,
	XIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import type { INode } from "../../../lib/schema/flow/node";
import { useBackend } from "../../../state/backend-state";
import type {
	IApplyFlowScriptResponse,
	IFlowScriptDiagnostic,
} from "../../../state/backend-state/board-state";
import { useSuppressFabBubble } from "../../../state/fab-suppression";
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
	FLOWSCRIPT_DIAGNOSTIC_OWNER,
	FLOWSCRIPT_LANGUAGE_ID,
	FLOWSCRIPT_THEME_DARK,
	FLOWSCRIPT_THEME_LIGHT,
	computeFlowScriptDiagnostics,
	defineFlowScriptThemes,
	registerFlowScriptLanguage,
	registerFlowScriptProviders,
} from "./flowscript-language";
import {
	canApplyFlowScript,
	shouldReloadFlowScriptAfterApply,
} from "./flowscript-panel-state";

const DESTRUCTIVE_BLOCK_PREFIX = "FlowScript edit would delete ";

const LINT_DEBOUNCE_MS = 300;

interface ApplyOptions {
	allowDeletions?: boolean;
	suppressBlockedToast?: boolean;
}

export interface FlowScriptPanelProps {
	appId: string;
	boardId: string;
	/** Defined when viewing an old board version — the editor becomes read-only. */
	version?: [number, number, number];
	/** Bump to re-render the script when the board changed (e.g. react-query dataUpdatedAt). */
	boardUpdatedAt?: number;
	catalogNodes?: INode[];
	onApplyFlowScript: (
		flowscript: string,
		options?: ApplyOptions,
	) => Promise<IApplyFlowScriptResponse | undefined>;
	onClose: () => void;
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

export function FlowScriptPanel({
	appId,
	boardId,
	version,
	boardUpdatedAt,
	catalogNodes,
	onApplyFlowScript,
	onClose,
}: Readonly<FlowScriptPanelProps>) {
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
	const [destructiveMessage, setDestructiveMessage] = useState<
		string | undefined
	>(undefined);

	const readOnly = typeof version !== "undefined";
	const dirty = text !== baseline;
	const applyState = {
		readOnly,
		dirty,
		applying,
		loading,
		boardChangedBehindEdits,
	};
	const canApply = canApplyFlowScript(applyState);
	const applyStateRef = useRef(applyState);
	applyStateRef.current = applyState;

	const dirtyRef = useRef(dirty);
	dirtyRef.current = dirty;
	const textRef = useRef(text);
	textRef.current = text;

	const catalogRef = useRef<INode[] | undefined>(catalogNodes);
	catalogRef.current = catalogNodes;
	const providersDisposable = useRef<{ dispose: () => void } | null>(null);
	const editorRef = useRef<Parameters<OnMount>[0] | null>(null);
	const monacoRef = useRef<Monaco | null>(null);

	// `version` is a fresh array reference every render; key on its stable string
	// form so load() (and the effects depending on it) don't re-fire in a loop.
	const versionKey = version?.join("_");
	const load = useCallback(async () => {
		setLoading(true);
		setLoadError(undefined);
		try {
			const parsedVersion = versionKey
				? (versionKey.split("_").map(Number) as [number, number, number])
				: undefined;
			const script = await backend.boardState.getFlowScript(
				appId,
				boardId,
				parsedVersion,
				true,
			);
			setText(script);
			setBaseline(script);
			setBoardChangedBehindEdits(false);
		} catch (error) {
			setLoadError(
				error instanceof Error ? error.message : "Failed to render FlowScript",
			);
		} finally {
			setLoading(false);
		}
	}, [backend, appId, boardId, versionKey]);

	// Initial load and reload on board/version switch.
	useEffect(() => {
		void load();
	}, [load]);

	// Board mutated elsewhere (canvas edits, collaborators): refresh in place unless
	// the user has unsaved text edits — then pause apply and offer an explicit refresh.
	// The initial value is swallowed so mount doesn't double-fetch alongside load().
	const lastBoardUpdateRef = useRef<number | undefined>(undefined);
	// biome-ignore lint/correctness/useExhaustiveDependencies: only board updates should trigger this
	useEffect(() => {
		if (typeof boardUpdatedAt === "undefined") return;
		if (typeof lastBoardUpdateRef.current === "undefined") {
			lastBoardUpdateRef.current = boardUpdatedAt;
			return;
		}
		if (lastBoardUpdateRef.current === boardUpdatedAt) return;
		lastBoardUpdateRef.current = boardUpdatedAt;
		if (dirtyRef.current) {
			setBoardChangedBehindEdits(true);
			return;
		}
		void load();
	}, [boardUpdatedAt]);

	useEffect(
		() => () => {
			providersDisposable.current?.dispose();
			providersDisposable.current = null;
		},
		[],
	);

	const runApply = useCallback(
		async (allowDeletions: boolean) => {
			if (!canApplyFlowScript(applyStateRef.current)) {
				if (applyStateRef.current.boardChangedBehindEdits) {
					toast.warning(
						"The board changed while you were editing. Refresh FlowScript before applying your draft.",
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
		[onApplyFlowScript, load],
	);

	const applyRef = useRef<() => void>(() => {});
	applyRef.current = () => {
		void runApply(false);
	};

	const handleEditorMount: OnMount = useCallback((editor, monaco) => {
		editorRef.current = editor;
		monacoRef.current = monaco;
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
		editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
			applyRef.current();
		});
		setEditorReady(true);
	}, []);

	// Realtime linting: instant client-side structural markers everywhere, plus authoritative
	// positioned diagnostics from the native parser where available (Flow-Like Studio / desktop).
	// biome-ignore lint/correctness/useExhaustiveDependencies: editorReady gates the first run once the editor mounts
	useEffect(() => {
		const monaco = monacoRef.current;
		const editor = editorRef.current;
		if (!monaco || !editor) return;
		const model = editor.getModel();
		if (!model) return;
		const source = text;
		const handle = setTimeout(async () => {
			const clientMarkers = computeFlowScriptDiagnostics(
				monaco,
				source,
				catalogRef.current,
			).markers;
			let nativeMarkers: unknown[] = [];
			try {
				const diagnostics = await backend.boardState.lintFlowScript?.(source);
				if (diagnostics) {
					nativeMarkers = diagnostics.map((diagnostic) =>
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
		}, LINT_DEBOUNCE_MS);
		return () => clearTimeout(handle);
	}, [text, catalogNodes, backend, editorReady]);

	const handleCopy = useCallback(async () => {
		await navigator.clipboard.writeText(textRef.current);
		toast.success("FlowScript copied to clipboard");
	}, []);

	const requestReload = useCallback(() => {
		if (dirtyRef.current) {
			setRefreshConfirmationOpen(true);
			return;
		}
		void load();
	}, [load]);

	const editorTheme = useMemo(
		() =>
			resolvedTheme === "dark" ? FLOWSCRIPT_THEME_DARK : FLOWSCRIPT_THEME_LIGHT,
		[resolvedTheme],
	);

	return (
		<div className="flex h-full min-h-0 w-full flex-col bg-background">
			<div className="flex items-center justify-between gap-2 border-b px-3 py-2">
				<div className="flex min-w-0 items-center gap-2">
					<FileCode2Icon className="h-4 w-4 shrink-0 text-primary" />
					<span className="truncate text-sm font-medium">FlowScript</span>
					{readOnly && (
						<Badge variant="secondary" className="text-[10px]">
							v{version?.join(".")} — read-only
						</Badge>
					)}
					{dirty && !readOnly && (
						<span
							className="h-2 w-2 shrink-0 rounded-full bg-primary"
							title="Unapplied changes"
						/>
					)}
				</div>
				<div className="flex items-center gap-1">
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
						<TooltipContent>Copy source</TooltipContent>
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
						<TooltipContent>Re-render from board</TooltipContent>
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

			{boardChangedBehindEdits && (
				<div
					role="alert"
					className="flex flex-wrap items-center justify-between gap-2 border-b bg-[color-mix(in_oklch,var(--primary)_8%,transparent)] px-3 py-2 text-xs text-muted-foreground"
				>
					<span className="flex min-w-0 items-center gap-2">
						<AlertTriangleIcon className="h-3.5 w-3.5 shrink-0 text-yellow-500" />
						The board changed while you were editing. Applying is paused until
						you refresh from the board.
					</span>
					<div className="flex shrink-0 items-center gap-1.5">
						<Button
							variant="ghost"
							size="sm"
							className="h-7 px-2 text-xs"
							onClick={() => void handleCopy()}
						>
							<CopyIcon className="mr-1 h-3 w-3" />
							Copy edits
						</Button>
						<Button
							variant="outline"
							size="sm"
							className="h-7 px-2 text-xs"
							disabled={loading || applying}
							onClick={requestReload}
						>
							<RefreshCcwIcon className="mr-1 h-3 w-3" />
							Refresh from board
						</Button>
					</div>
				</div>
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
							Retry
						</Button>
					</div>
				) : (
					<Editor
						height="100%"
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
			</div>

			{diagnostics.length > 0 && (
				<div className="max-h-28 shrink-0 overflow-y-auto border-t px-3 py-2">
					<div className="mb-1 flex items-center justify-between">
						<span className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
							<AlertTriangleIcon className="h-3 w-3 text-yellow-500" />
							{diagnostics.length} warning{diagnostics.length === 1 ? "" : "s"}
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
							? "Board changed — refresh before applying"
							: dirty
								? "Unapplied changes — ⌘S to apply"
								: "In sync with board"}
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
							}}
						>
							<Undo2Icon className="mr-1 h-3.5 w-3.5" />
							Reset
						</Button>
						<Button
							size="sm"
							className="h-7"
							disabled={!canApply}
							onClick={() => void runApply(false)}
						>
							{applying ? (
								<Loader2Icon className="mr-1 h-3.5 w-3.5 animate-spin" />
							) : null}
							Apply to board
						</Button>
					</div>
				</div>
			)}

			<AlertDialog
				open={refreshConfirmationOpen}
				onOpenChange={setRefreshConfirmationOpen}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							Refresh FlowScript from the board?
						</AlertDialogTitle>
						<AlertDialogDescription>
							This replaces the current editor text with the latest board state.
							Copy your edits first if you want to reapply them to the refreshed
							script.
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>Keep editing</AlertDialogCancel>
						<Button variant="outline" onClick={() => void handleCopy()}>
							<CopyIcon className="mr-1 h-3.5 w-3.5" />
							Copy edits
						</Button>
						<AlertDialogAction
							onClick={() => {
								setRefreshConfirmationOpen(false);
								void load();
							}}
						>
							Refresh and replace
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
							This edit deletes existing board items
						</AlertDialogTitle>
						<AlertDialogDescription className="break-words">
							{destructiveMessage}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>Keep everything</AlertDialogCancel>
						<AlertDialogAction
							onClick={() => {
								setDestructiveMessage(undefined);
								void runApply(true);
							}}
						>
							Apply with deletions
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}
