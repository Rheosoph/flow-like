"use client";

import Editor, { type Monaco } from "@monaco-editor/react";
import { CheckCircle2, CopyIcon, FileCode2Icon, XIcon } from "lucide-react";
import { useTheme } from "next-themes";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { FLOW_KEY_OPT_OUT_CLASS } from "../../lib/monaco-key-guard";
import {
	FLOWSCRIPT_LANGUAGE_ID,
	FLOWSCRIPT_THEME_DARK,
	FLOWSCRIPT_THEME_LIGHT,
	setupFlowScriptEditor,
} from "../flow/flowscript/flowscript-language";
import { Button } from "../ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

export function formatLineCount(source: string): string {
	const lines = source ? source.split("\n").length : 0;
	return `${lines} line${lines === 1 ? "" : "s"}`;
}

interface FlowScriptWorkspacePanelProps {
	source: string;
	/** Optional workspace status chip (e.g. "drafting", "validation_errors"). */
	status?: string;
	/** When provided, renders a close button in the panel header. */
	onClose?: () => void;
	/**
	 * Fill the parent container instead of sitting as a fixed-width sidebar. Used when the panel
	 * replaces the chat in a narrow surface (e.g. the docked overlay) — the sidebar's `min-w`
	 * would otherwise overflow a small window and force horizontal scroll.
	 */
	fill?: boolean;
}

/**
 * Read-only Monaco view of the copilot's FlowScript workspace. Shared by the board FlowPilot and
 * the global assistant so both surfaces present generated FlowScript identically.
 */
export const FlowScriptWorkspacePanel = memo(function FlowScriptWorkspacePanel({
	source,
	status,
	onClose,
	fill = false,
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
			setupFlowScriptEditor(monaco, isDark);
		},
		[isDark],
	);

	useEffect(() => {
		if (!monacoRef.current) return;
		setupFlowScriptEditor(monacoRef.current, isDark);
	}, [isDark]);

	return (
		<aside
			className={
				fill
					? "flex h-full min-h-0 w-full flex-1 flex-col bg-muted/20"
					: "flex h-[42dvh] min-h-[260px] w-full shrink-0 flex-col border-t border-border/30 bg-muted/20 md:h-full md:min-h-0 md:w-[48%] md:min-w-[420px] md:max-w-[660px] md:border-l md:border-t-0"
			}
		>
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
					{status && (
						<div
							className={`rounded-full px-2 py-0.5 text-[10px] font-medium ${
								status === "validation_errors"
									? "bg-red-500/15 text-red-500"
									: "bg-primary/10 text-primary"
							}`}
						>
							{status.replaceAll("_", " ")}
						</div>
					)}
				</div>
				<div className="flex items-center gap-1">
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
					{onClose && (
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="h-7 w-7 shrink-0 rounded-md"
							aria-label="Close FlowScript panel"
							onClick={onClose}
						>
							<XIcon className="h-4 w-4" />
						</Button>
					)}
				</div>
			</div>
			<div className="min-h-0 flex-1 bg-linear-to-b from-muted/20 to-background/50 p-3">
				<div className="h-full min-h-0 overflow-hidden rounded-xl border border-border/45 bg-card shadow-[0_18px_45px_rgba(15,23,42,0.08)] ring-1 ring-black/[0.02] dark:shadow-black/25 dark:ring-white/[0.03]">
					<Editor
						beforeMount={handleBeforeMount}
						className={FLOW_KEY_OPT_OUT_CLASS}
						height="100%"
						language={FLOWSCRIPT_LANGUAGE_ID}
						theme={isDark ? FLOWSCRIPT_THEME_DARK : FLOWSCRIPT_THEME_LIGHT}
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
