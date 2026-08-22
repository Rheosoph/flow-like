"use client";

import Editor, { type Monaco } from "@monaco-editor/react";
import { EyeIcon, SaveIcon } from "lucide-react";
import { useTheme } from "next-themes";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { FLOW_KEY_OPT_OUT_CLASS } from "../../lib/monaco-key-guard";
import { Button } from "./button";
import { TextEditor } from "./text-editor";

const LANGUAGE_MAP: Record<string, string> = {
	js: "javascript",
	jsx: "javascript",
	ts: "typescript",
	tsx: "typescript",
	json: "json",
	yml: "yaml",
	yaml: "yaml",
	py: "python",
	java: "java",
	c: "c",
	cpp: "cpp",
	h: "c",
	hpp: "cpp",
	cs: "csharp",
	go: "go",
	rb: "ruby",
	php: "php",
	swift: "swift",
	kt: "kotlin",
	rs: "rust",
	html: "html",
	css: "css",
	scss: "scss",
	sass: "sass",
	less: "less",
	xml: "xml",
	sql: "sql",
	sh: "shell",
	bash: "shell",
	toml: "toml",
	vue: "vue",
	svelte: "svelte",
	md: "markdown",
	mdx: "markdown",
};

function getLanguageFromFileName(fileName: string): string {
	const extension = fileName.split(".").pop()?.toLowerCase() ?? "";
	return LANGUAGE_MAP[extension] || "plaintext";
}

function isMarkdownFile(fileName: string): boolean {
	const extension = fileName.split(".").pop()?.toLowerCase() ?? "";
	return extension === "md" || extension === "mdx";
}

// Inject CSS to style Monaco editor with Flow-like background and foreground
function injectMonacoStyles() {
	if (typeof window === "undefined") return;

	const styleId = "monaco-flow-like-theme";
	if (document.getElementById(styleId)) return;

	const style = document.createElement("style");
	style.id = styleId;
	style.textContent = `
		/* Monaco Editor - Only override background and foreground */
		.monaco-editor .margin,
		.monaco-editor-background,
		.monaco-editor .inputarea.ime-input {
			background-color: var(--background) !important;
		}

		.monaco-editor .view-line {
			color: var(--foreground) !important;
		}
	`;
	document.head.appendChild(style);
}

function defineFlowLikeTheme(monaco: Monaco, isDark: boolean) {
	// Use Monaco's built-in theme (vs-dark has good syntax highlighting)
	// CSS will override only background and foreground
	monaco.editor.setTheme(isDark ? "vs-dark" : "vs");
}

export function MonacoFileEditor({
	fileName,
	initialContent,
	editable = true,
	onSave,
}: Readonly<{
	fileName: string;
	initialContent: string;
	editable?: boolean;
	onSave?: (content: string) => Promise<void>;
}>) {
	const [content, setContent] = useState(initialContent);
	const [isSaving, setIsSaving] = useState(false);
	const [hasChanges, setHasChanges] = useState(false);
	const [isPreviewMode, setIsPreviewMode] = useState(false);
	const [isMonacoReady, setIsMonacoReady] = useState(false);
	const { resolvedTheme } = useTheme();
	const monacoRef = useRef<Monaco | null>(null);
	const language = getLanguageFromFileName(fileName);
	const isMarkdown = isMarkdownFile(fileName);

	// Inject CSS styles once
	useEffect(() => {
		injectMonacoStyles();
	}, []);

	useEffect(() => {
		setContent(initialContent);
		setHasChanges(false);
	}, [initialContent, fileName]);

	const handleEditorMount = useCallback(
		(monaco: Monaco) => {
			monacoRef.current = monaco;
			defineFlowLikeTheme(monaco, resolvedTheme === "dark");
			setIsMonacoReady(true);
		},
		[resolvedTheme],
	);

	useEffect(() => {
		if (monacoRef.current && isMonacoReady) {
			defineFlowLikeTheme(monacoRef.current, resolvedTheme === "dark");
		}
	}, [resolvedTheme, isMonacoReady]);

	const handleEditorChange = useCallback(
		(value: string | undefined) => {
			const newContent = value ?? "";
			setContent(newContent);
			setHasChanges(newContent !== initialContent);
		},
		[initialContent],
	);

	const handleSave = useCallback(async () => {
		if (!onSave || !hasChanges) return;

		setIsSaving(true);
		try {
			await onSave(content);
			setHasChanges(false);
			toast.success("File saved successfully");
		} catch (error) {
			console.error("Failed to save file:", error);
			toast.error("Failed to save file");
		} finally {
			setIsSaving(false);
		}
	}, [onSave, content, hasChanges]);

	useEffect(() => {
		const handleKeyDown = (e: KeyboardEvent) => {
			if ((e.metaKey || e.ctrlKey) && e.key === "s") {
				e.preventDefault();
				if (hasChanges && editable) {
					handleSave();
				}
			}
		};

		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [handleSave, hasChanges, editable]);

	return (
		<div className="flex flex-col h-full w-full">
			{editable && (hasChanges || isMarkdown) && (
				<div className="flex items-center justify-between p-2 border-b bg-muted/50">
					<span className="text-sm text-muted-foreground">
						{hasChanges ? "Unsaved changes" : ""}
					</span>
					<div className="flex items-center gap-2">
						{isMarkdown && (
							<Button
								size="sm"
								variant={isPreviewMode ? "default" : "outline"}
								onClick={() => setIsPreviewMode(!isPreviewMode)}
								className="h-7 gap-2"
							>
								<EyeIcon className="h-3 w-3" />
								{isPreviewMode ? "Edit" : "Preview"}
							</Button>
						)}
						{hasChanges && (
							<Button
								size="sm"
								onClick={handleSave}
								disabled={isSaving}
								className="h-7 gap-2"
							>
								<SaveIcon className="h-3 w-3" />
								{isSaving ? "Saving..." : "Save"}
							</Button>
						)}
					</div>
				</div>
			)}
			<div className="flex-1 min-h-0">
				{isPreviewMode && isMarkdown ? (
					<TextEditor
						initialContent={content}
						isMarkdown={true}
						editable={false}
					/>
				) : (
					<Editor
						className={FLOW_KEY_OPT_OUT_CLASS}
						height="100%"
						language={language}
						value={content}
						onChange={handleEditorChange}
						theme={resolvedTheme === "dark" ? "vs-dark" : "vs"}
						beforeMount={handleEditorMount}
						options={{
							readOnly: !editable,
							minimap: { enabled: false },
							fontSize: 14,
							lineNumbers: "on",
							scrollBeyondLastLine: false,
							automaticLayout: true,
							wordWrap: "on",
							wrappingIndent: "indent",
							tabSize: 2,
						}}
					/>
				)}
			</div>
		</div>
	);
}
