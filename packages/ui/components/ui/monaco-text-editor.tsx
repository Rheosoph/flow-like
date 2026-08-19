"use client";

import { i18n as i18next } from "@flow-like/locales";
import Editor, { type Monaco } from "@monaco-editor/react";
import { useTheme } from "next-themes";
import { useCallback, useEffect, useRef, useState } from "react";

function injectMonacoStyles() {
	if (typeof window === "undefined") return;

	const styleId = "monaco-flow-like-theme";
	if (document.getElementById(styleId)) return;

	const style = document.createElement("style");
	style.id = styleId;
	style.textContent = `.monaco-text-editor-wrapper { background: linear-gradient(to bottom, hsl(var(--muted) / 0.3), hsl(var(--muted) / 0.1)); } .monaco-text-editor-wrapper .monaco-editor { border-radius: 0.5rem; } .monaco-text-editor-wrapper .monaco-editor .margin, .monaco-text-editor-wrapper .monaco-editor-background, .monaco-text-editor-wrapper .monaco-editor .inputarea.ime-input { background-color: transparent !important; } .monaco-text-editor-wrapper .monaco-editor .view-line { color: var(--foreground) !important; font-weight: 400; letter-spacing: 0.01em; } .monaco-text-editor-wrapper .monaco-editor .margin { background: transparent !important; } .monaco-text-editor-wrapper .monaco-editor .line-numbers { color: hsl(var(--muted-foreground) / 0.6) !important; font-size: 0.75rem; } .monaco-text-editor-wrapper .monaco-editor .current-line { border: none !important; } .monaco-text-editor-wrapper .monaco-editor .selected-text { background-color: hsl(var(--accent) / 0.3) !important; } .monaco-text-editor-wrapper .monaco-editor .view-overlays .current-line { background: linear-gradient(to right, hsl(var(--accent) / 0.05), transparent) !important; } .monaco-text-editor-wrapper .monaco-editor .cursors-layer .cursor { background-color: hsl(var(--primary)) !important; width: 2px !important; } .monaco-text-editor-wrapper .monaco-scrollable-element > .scrollbar > .slider { background: hsl(var(--muted-foreground) / 0.3) !important; border-radius: 4px; } .monaco-text-editor-wrapper .monaco-scrollable-element > .scrollbar > .slider:hover { background: hsl(var(--muted-foreground) / 0.5) !important; }`;
	document.head.appendChild(style);
}

function defineTheme(monaco: Monaco, isDark: boolean) {
	monaco.editor.setTheme(isDark ? "vs-dark" : "vs");
}

export function MonacoTextEditor({
	value,
	onChange,
	disabled = false,
	placeholder = i18next.t('enterText', 'Enter text...'),
	height = "400px",
}: Readonly<{
	value: string;
	onChange: (value: string) => void;
	disabled?: boolean;
	placeholder?: string;
	height?: string;
}>) {
	const [isMonacoReady, setIsMonacoReady] = useState(false);
	const { resolvedTheme } = useTheme();
	const monacoRef = useRef<Monaco | null>(null);

	useEffect(() => {
		injectMonacoStyles();
	}, []);

	const handleEditorMount = useCallback(
		(monaco: Monaco) => {
			monacoRef.current = monaco;
			defineTheme(monaco, resolvedTheme === "dark");
			setIsMonacoReady(true);
		},
		[resolvedTheme],
	);

	useEffect(() => {
		if (monacoRef.current && isMonacoReady) {
			defineTheme(monacoRef.current, resolvedTheme === "dark");
		}
	}, [resolvedTheme, isMonacoReady]);

	const handleEditorChange = useCallback(
		(newValue: string | undefined) => {
			onChange(newValue ?? "");
		},
		[onChange],
	);

	return (
		<div className="monaco-text-editor-wrapper relative rounded-lg border border-input shadow-md transition-all duration-200 focus-within:ring-2 focus-within:ring-primary focus-within:ring-offset-2 focus-within:shadow-lg focus-within:border-primary/50 overflow-hidden px-6">
			<Editor
				height={height}
				language="plaintext"
				value={value}
				onChange={handleEditorChange}
				theme={resolvedTheme === "dark" ? "vs-dark" : "vs"}
				beforeMount={handleEditorMount}
				options={{
					readOnly: disabled,
					minimap: { enabled: false },
					fontSize: 14,
					fontFamily:
						"'SF Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace",
					fontLigatures: true,
					lineNumbers: "off",
					scrollBeyondLastLine: false,
					automaticLayout: true,
					wordWrap: "on",
					wrappingIndent: "indent",
					tabSize: 2,
					lineHeight: 24,
					padding: { top: 20, bottom: 20 },
					scrollbar: {
						vertical: "auto",
						horizontal: "auto",
						verticalScrollbarSize: 10,
						horizontalScrollbarSize: 10,
						useShadows: false,
					},
					folding: false,
					glyphMargin: false,
					lineDecorationsWidth: 0,
					lineNumbersMinChars: 4,
					renderLineHighlight: "line",
					overviewRulerLanes: 0,
					hideCursorInOverviewRuler: true,
					overviewRulerBorder: false,
					placeholder,
					cursorBlinking: "smooth",
					cursorSmoothCaretAnimation: "on",
					smoothScrolling: true,
					contextmenu: true,
					quickSuggestions: false,
					suggestOnTriggerCharacters: false,
					acceptSuggestionOnEnter: "off",
					tabCompletion: "off",
					wordBasedSuggestions: "off",
				}}
			/>
		</div>
	);
}
