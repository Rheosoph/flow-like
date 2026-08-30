"use client";

import Editor, { type Monaco, type OnMount } from "@monaco-editor/react";
import { Maximize2, Minimize2 } from "lucide-react";
import { useTheme } from "next-themes";
import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { FLOW_KEY_OPT_OUT_CLASS, cn } from "../../lib";
import { Button } from "./button";

function injectMonacoCodeStyles() {
	if (typeof window === "undefined") return;

	const styleId = "monaco-code-editor-theme";
	if (document.getElementById(styleId)) return;

	const style = document.createElement("style");
	style.id = styleId;
	style.textContent = `.monaco-code-editor-wrapper .monaco-editor .line-numbers { font-size: 0.75rem; } .monaco-code-editor-wrapper .monaco-scrollable-element > .scrollbar > .slider { background: hsl(var(--muted-foreground) / 0.3) !important; border-radius: 4px; } .monaco-code-editor-wrapper .monaco-scrollable-element > .scrollbar > .slider:hover { background: hsl(var(--muted-foreground) / 0.5) !important; } /* Fullscreen mode */ .monaco-code-editor-fullscreen { position: fixed !important; inset: 0 !important; z-index: 50 !important; height: 100vh !important; width: 100vw !important; border-radius: 0 !important; }`;
	document.head.appendChild(style);
}

export type MonacoLanguage =
	| "css"
	| "json"
	| "javascript"
	| "typescript"
	| "html"
	| "plaintext";

export interface MonacoCodeEditorProps {
	value: string;
	onChange: (value: string) => void;
	language?: MonacoLanguage;
	disabled?: boolean;
	height?: string;
	className?: string;
	showLineNumbers?: boolean;
	showMinimap?: boolean;
	allowFullscreen?: boolean;
	autoFocus?: boolean;
}

export function MonacoCodeEditor({
	value,
	onChange,
	language = "plaintext",
	disabled = false,
	height = "200px",
	className,
	showLineNumbers = true,
	showMinimap = false,
	allowFullscreen = false,
	autoFocus = true,
}: Readonly<MonacoCodeEditorProps>) {
	const [isFullscreen, setIsFullscreen] = useState(false);
	const { resolvedTheme } = useTheme();
	const monacoRef = useRef<Monaco | null>(null);

	useEffect(() => {
		injectMonacoCodeStyles();
	}, []);

	// Handle escape key to exit fullscreen
	useEffect(() => {
		if (!isFullscreen) return;

		const handleKeyDown = (e: KeyboardEvent) => {
			if (e.key === "Escape") {
				setIsFullscreen(false);
			}
		};

		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [isFullscreen]);

	// Update theme when resolvedTheme changes
	useEffect(() => {
		if (monacoRef.current) {
			monacoRef.current.editor.setTheme(
				resolvedTheme === "dark" ? "vs-dark" : "vs",
			);
		}
	}, [resolvedTheme]);

	const handleEditorMount: OnMount = useCallback(
		(editor, monaco) => {
			monacoRef.current = monaco;
			if (autoFocus && !disabled) editor.focus();
		},
		[autoFocus, disabled],
	);

	const handleEditorChange = useCallback(
		(newValue: string | undefined) => {
			onChange(newValue ?? "");
		},
		[onChange],
	);

	const editorContent = (
		<div
			className={cn(
				"monaco-code-editor-wrapper relative rounded-md border bg-muted/30 overflow-hidden",
				isFullscreen && "monaco-code-editor-fullscreen bg-background",
				className,
			)}
		>
			{allowFullscreen && (
				<Button
					variant="ghost"
					size="icon"
					className="absolute top-2 right-2 z-10 h-7 w-7 bg-background/80 hover:bg-background"
					onClick={() => setIsFullscreen(!isFullscreen)}
				>
					{isFullscreen ? (
						<Minimize2 className="h-4 w-4" />
					) : (
						<Maximize2 className="h-4 w-4" />
					)}
				</Button>
			)}
			<Editor
				className={FLOW_KEY_OPT_OUT_CLASS}
				height={isFullscreen ? "100vh" : height}
				language={language}
				value={value}
				onChange={handleEditorChange}
				theme={resolvedTheme === "dark" ? "vs-dark" : "vs"}
				onMount={handleEditorMount}
				options={{
					readOnly: disabled,
					minimap: { enabled: showMinimap || isFullscreen },
					fontSize: isFullscreen ? 14 : 12,
					fontFamily:
						"'SF Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace",
					fontLigatures: true,
					lineNumbers: showLineNumbers ? "on" : "off",
					scrollBeyondLastLine: false,
					automaticLayout: true,
					wordWrap: "on",
					wrappingIndent: "indent",
					tabSize: 2,
					lineHeight: 18,
					padding: { top: 8, bottom: 8 },
					scrollbar: {
						vertical: "auto",
						horizontal: "auto",
						verticalScrollbarSize: 8,
						horizontalScrollbarSize: 8,
						useShadows: false,
					},
					folding: true,
					glyphMargin: false,
					lineDecorationsWidth: 4,
					lineNumbersMinChars: 3,
					renderLineHighlight: "line",
					overviewRulerLanes: 0,
					hideCursorInOverviewRuler: true,
					overviewRulerBorder: false,
					cursorBlinking: "smooth",
					cursorSmoothCaretAnimation: "on",
					cursorStyle: "line",
					cursorWidth: 2,
					smoothScrolling: true,
					contextmenu: true,
					quickSuggestions: true,
					suggestOnTriggerCharacters: true,
					acceptSuggestionOnEnter: "on",
					tabCompletion: "on",
					formatOnPaste: true,
					formatOnType: true,
				}}
			/>
		</div>
	);

	// Use portal for fullscreen to escape any parent stacking contexts
	if (isFullscreen && typeof document !== "undefined") {
		return (
			<>
				{/* Placeholder to maintain layout when fullscreen */}
				<div
					className={cn("rounded-md border bg-muted/30", className)}
					style={{ height }}
				/>
				{createPortal(editorContent, document.body)}
			</>
		);
	}

	return editorContent;
}
