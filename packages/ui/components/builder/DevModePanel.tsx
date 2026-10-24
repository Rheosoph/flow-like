"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import type { OnMount } from "@monaco-editor/react";
import { AlertCircle, Check, Code, Copy, X } from "lucide-react";
import {
	type MouseEvent,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
} from "../ui/dialog";
import { MonacoCodeEditor } from "../ui/monaco-code-editor";
import { useBuilder } from "./BuilderContext";

export interface DevModePanelProps {
	className?: string;
}

export function DevModePanel({ className }: DevModePanelProps) {
	const { t } = useTranslation("flow");
	const { devMode, setDevMode, getRawJson, setRawJson } = useBuilder();
	const [json, setJson] = useState("");
	const [error, setError] = useState<string | null>(null);
	const [copied, setCopied] = useState(false);
	const editorRef = useRef<Parameters<OnMount>[0] | null>(null);
	const returnFocusRef = useRef<HTMLElement | null>(null);

	const handleEditorMount: OnMount = useCallback((editor) => {
		editorRef.current = editor;
	}, []);

	const handleMouseDown = useCallback((event: MouseEvent<HTMLDivElement>) => {
		if (
			event.button !== 0 ||
			event.defaultPrevented ||
			!(event.target instanceof Element) ||
			event.target.closest(
				'.monaco-editor, button, a, input, textarea, select, [contenteditable="true"]',
			)
		) {
			return;
		}

		// Blank dialog areas should focus the editor without starting a page selection.
		event.preventDefault();
		editorRef.current?.focus();
	}, []);

	// Load JSON when panel opens
	useEffect(() => {
		if (devMode) {
			setJson(getRawJson());
			setError(null);
		}
	}, [devMode, getRawJson]);

	const handleCopy = useCallback(async () => {
		try {
			await navigator.clipboard.writeText(json);
			setCopied(true);
			setTimeout(() => setCopied(false), 2000);
		} catch (e) {
			console.error("Failed to copy:", e);
		}
	}, [json]);

	const handleApply = useCallback(() => {
		const success = setRawJson(json);
		if (success) {
			setError(null);
			setDevMode(false);
			toast.success(t("devModeChangesApplied", "Changes applied"));
		} else {
			setError(
				t(
					"devModeApplyFailed",
					"Could not apply changes. Check the JSON syntax and include a components array with an id and component for each item.",
				),
			);
		}
	}, [json, setRawJson, setDevMode, t]);

	const handleFormat = useCallback(() => {
		try {
			const parsed = JSON.parse(json);
			setJson(JSON.stringify(parsed, null, 2));
			setError(null);
		} catch (e) {
			setError("Invalid JSON: Cannot format");
		}
	}, [json]);

	if (!devMode) return null;

	return (
		<Dialog open={devMode} onOpenChange={setDevMode}>
			<DialogContent
				showCloseButton={false}
				overlayClassName="bg-background"
				className={cn(
					"h-[calc(100dvh-2rem)] max-w-[calc(100%-2rem)] gap-0 overflow-hidden rounded-lg bg-background p-0 sm:max-w-[calc(100%-2rem)]",
					className,
				)}
				onOpenAutoFocus={() => {
					returnFocusRef.current = document.activeElement as HTMLElement | null;
				}}
				onCloseAutoFocus={(event) => {
					event.preventDefault();
					if (returnFocusRef.current?.isConnected) {
						returnFocusRef.current.focus();
					}
					editorRef.current = null;
				}}
				onPointerDownOutside={(event) => event.preventDefault()}
				onEscapeKeyDown={(event) => event.preventDefault()}
				onMouseDown={handleMouseDown}
			>
				{/* Header */}
				<div className="flex shrink-0 items-center justify-between px-4 py-3 border-b">
					<div className="flex items-center gap-2">
						<Code className="h-5 w-5" />
						<DialogTitle className="text-base leading-normal">
							{t("devModeRawJsonEditor", "Dev Mode - Raw JSON Editor")}
						</DialogTitle>
					</div>
					<div className="flex items-center gap-2">
						<Button variant="outline" size="sm" onClick={handleFormat}>
							{t("format", "Format")}
						</Button>
						<Button variant="outline" size="sm" onClick={handleCopy}>
							{copied ? (
								<>
									<Check className="h-4 w-4 mr-1" />
									{t("copied", "Copied")}
								</>
							) : (
								<>
									<Copy className="h-4 w-4 mr-1" />
									{t("copy", "Copy")}
								</>
							)}
						</Button>
						<Button variant="default" size="sm" onClick={handleApply}>
							{t("applyChanges", "Apply Changes")}
						</Button>
						<Button
							variant="ghost"
							size="icon"
							aria-label={t("close", "Close")}
							onClick={() => setDevMode(false)}
						>
							<X className="h-4 w-4" />
						</Button>
					</div>
				</div>

				{/* Error banner */}
				{error && (
					<div
						role="alert"
						className="shrink-0 px-4 py-2 bg-destructive/10 text-destructive flex items-center gap-2"
					>
						<AlertCircle className="h-4 w-4" />
						<span className="text-sm">{error}</span>
					</div>
				)}

				{/* Editor */}
				<div className="min-h-0 flex-1 overflow-hidden">
					<MonacoCodeEditor
						value={json}
						onChange={(value) => {
							setJson(value);
							setError(null);
						}}
						language="json"
						height="100%"
						className="h-full rounded-none border-0"
						onMount={handleEditorMount}
						showMinimap={true}
					/>
				</div>

				{/* Footer */}
				<DialogDescription className="shrink-0 px-4 py-2 border-t text-xs">
					{t(
						"editTheRawJsonStructureOfYourWidgetTheStructureIncludes",
						"Edit the raw JSON structure of your widget. The structure includes",
					)}{" "}
					<Trans i18nKey="codeClassnamebgmutedPx1RoundedcomponentscodeArrayOfSurfacecomponentAnd">
						<code className="bg-muted px-1 rounded">components</code> (array of
						SurfaceComponent) and
					</Trans>{" "}
					<Trans i18nKey="codeClassnamebgmutedPx1RoundedwidgetrefscodeWidgetDefinitions">
						<code className="bg-muted px-1 rounded">widgetRefs</code> (widget
						definitions).
					</Trans>
				</DialogDescription>
			</DialogContent>
		</Dialog>
	);
}
