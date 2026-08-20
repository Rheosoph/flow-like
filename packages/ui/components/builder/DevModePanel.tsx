"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { AlertCircle, Check, Code, Copy, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { cn } from "../../lib";
import { Button } from "../ui/button";
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
	const [saved, setSaved] = useState(false);

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

	const handleSave = useCallback(() => {
		const success = setRawJson(json);
		if (success) {
			setError(null);
			setSaved(true);
			setTimeout(() => setSaved(false), 2000);
		} else {
			setError("Invalid JSON structure. Check console for details.");
		}
	}, [json, setRawJson]);

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
		<div
			className={cn(
				"fixed inset-0 z-50 bg-background/80 backdrop-blur-sm",
				className,
			)}
		>
			<div className="fixed inset-4 bg-background border rounded-lg shadow-xl flex flex-col">
				{/* Header */}
				<div className="flex items-center justify-between px-4 py-3 border-b">
					<div className="flex items-center gap-2">
						<Code className="h-5 w-5" />
						<h2 className="font-semibold">
							{t("devModeRawJsonEditor", "Dev Mode - Raw JSON Editor")}
						</h2>
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
						<Button variant="default" size="sm" onClick={handleSave}>
							{saved ? (
								<>
									<Check className="h-4 w-4 mr-1" />
									{t("saved", "Saved")}
								</>
							) : (
								t("applyChanges", "Apply Changes")
							)}
						</Button>
						<Button
							variant="ghost"
							size="icon"
							onClick={() => setDevMode(false)}
						>
							<X className="h-4 w-4" />
						</Button>
					</div>
				</div>

				{/* Error banner */}
				{error && (
					<div className="px-4 py-2 bg-destructive/10 text-destructive flex items-center gap-2">
						<AlertCircle className="h-4 w-4" />
						<span className="text-sm">{error}</span>
					</div>
				)}

				{/* Editor */}
				<div className="flex-1 overflow-hidden">
					<MonacoCodeEditor
						value={json}
						onChange={(value) => {
							setJson(value);
							setError(null);
						}}
						language="json"
						height="calc(100vh - 180px)"
						showMinimap={true}
					/>
				</div>

				{/* Footer */}
				<div className="px-4 py-2 border-t text-xs text-muted-foreground">
					<p>
						{t(
							"editTheRawJsonStructureOfYourWidgetTheStructureIncludes",
							"Edit the raw JSON structure of your widget. The structure includes",
						)}{" "}
						<Trans i18nKey="codeClassnamebgmutedPx1RoundedcomponentscodeArrayOfSurfacecomponentAnd">
							<code className="bg-muted px-1 rounded">components</code> (array
							of SurfaceComponent) and
						</Trans>{" "}
						<Trans i18nKey="codeClassnamebgmutedPx1RoundedwidgetrefscodeWidgetDefinitions">
							<code className="bg-muted px-1 rounded">widgetRefs</code> (widget
							definitions).
						</Trans>
					</p>
				</div>
			</div>
		</div>
	);
}
