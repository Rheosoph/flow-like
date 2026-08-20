"use client";

import { useTranslation } from "@flow-like/locales";
import Editor, { type Monaco, type OnMount } from "@monaco-editor/react";
import { useTheme } from "next-themes";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	SQL_DIAGNOSTIC_OWNER,
	type SqlCatalog,
	computeSqlDiagnostics,
	ensureSqlProviders,
	setActiveSqlCatalog,
} from "./sql-language";

const DIAGNOSTIC_DEBOUNCE_MS = 300;

export interface SqlEditorProps {
	value: string;
	onChange: (value: string) => void;
	catalog: SqlCatalog;
	readOnly?: boolean;
	onRun?: () => void;
	onCursorChange?: (position: { line: number; column: number }) => void;
	height?: string;
}

export function SqlEditor({
	value,
	onChange,
	catalog,
	readOnly,
	onRun,
	onCursorChange,
	height = "100%",
}: Readonly<SqlEditorProps>) {
	const { t } = useTranslation("common");
	const { resolvedTheme } = useTheme();
	const editorRef = useRef<Parameters<OnMount>[0] | null>(null);
	const monacoRef = useRef<Monaco | null>(null);
	const onRunRef = useRef(onRun);
	onRunRef.current = onRun;
	const onCursorRef = useRef(onCursorChange);
	onCursorRef.current = onCursorChange;
	const [editorReady, setEditorReady] = useState(false);
	const reducedMotion =
		typeof window !== "undefined" &&
		typeof window.matchMedia === "function" &&
		window.matchMedia("(prefers-reduced-motion: reduce)").matches;

	// Keep the module-level catalog in sync so the shared completion provider
	// always reads the latest tables/columns/params without re-registering.
	useEffect(() => {
		setActiveSqlCatalog(catalog);
	}, [catalog]);

	const handleMount: OnMount = useCallback((editor, monaco) => {
		editorRef.current = editor;
		monacoRef.current = monaco;
		ensureSqlProviders(monaco);
		editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
			onRunRef.current?.();
		});
		editor.onDidChangeCursorPosition((event) => {
			onCursorRef.current?.({
				line: event.position.lineNumber,
				column: event.position.column,
			});
		});
		setEditorReady(true);
	}, []);

	// biome-ignore lint/correctness/useExhaustiveDependencies: editorReady gates the first run once mounted
	useEffect(() => {
		const monaco = monacoRef.current;
		const editor = editorRef.current;
		if (!monaco || !editor) return;
		const model = editor.getModel();
		if (!model) return;
		const source = value;
		const handle = setTimeout(() => {
			if (editor.getModel() !== model || model.getValue() !== source) return;
			const { markers } = computeSqlDiagnostics(monaco, source, catalog);
			monaco.editor.setModelMarkers(
				model,
				SQL_DIAGNOSTIC_OWNER,
				markers as unknown as Parameters<
					typeof monaco.editor.setModelMarkers
				>[2],
			);
		}, DIAGNOSTIC_DEBOUNCE_MS);
		return () => clearTimeout(handle);
	}, [value, catalog, editorReady]);

	return (
		<Editor
			height={height}
			language="sql"
			value={value}
			onChange={(next) => onChange(next ?? "")}
			theme={resolvedTheme === "dark" ? "vs-dark" : "vs"}
			onMount={handleMount}
			options={{
				readOnly,
				ariaLabel: t("sqlQueryEditor", "SQL query editor"),
				minimap: { enabled: false },
				fontSize: 13,
				fontFamily:
					"'SF Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace",
				fontLigatures: true,
				scrollBeyondLastLine: false,
				automaticLayout: true,
				wordWrap: "on",
				tabSize: 2,
				padding: { top: 10, bottom: 10 },
				renderLineHighlight: "line",
				smoothScrolling: !reducedMotion,
				cursorSmoothCaretAnimation: reducedMotion ? "off" : "on",
				quickSuggestions: true,
				suggestOnTriggerCharacters: true,
				tabCompletion: "on",
				parameterHints: { enabled: true },
				lineNumbers: "on",
			}}
		/>
	);
}
