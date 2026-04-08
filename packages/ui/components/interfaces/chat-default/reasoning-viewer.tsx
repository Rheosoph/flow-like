"use client";

import { BrainCircuit, ChevronDown, ChevronUp } from "lucide-react";
import { Suspense, lazy, useMemo, useState } from "react";
import { Button } from "../../ui/button";

const TextEditor = lazy(() =>
	import("../../ui/text-editor").then((m) => ({ default: m.TextEditor })),
);

interface ReasoningViewerProps {
	reasoning: string;
	defaultExpanded?: boolean;
	compact?: boolean;
}

function hasStructuredMarkdown(reasoning: string): boolean {
	return reasoning
		.split(/\r?\n/)
		.map((line) => line.trim())
		.filter(Boolean)
		.some(
			(line) =>
				line.startsWith("```") ||
				/^#{1,6}\s/.test(line) ||
				/^[-*+]\s/.test(line) ||
				/^\d+\.\s/.test(line) ||
				/^>\s/.test(line) ||
				/^\|.*\|$/.test(line),
		);
}

function looksLikeTokenizedReasoning(reasoning: string): boolean {
	const lines = reasoning
		.split(/\r?\n/)
		.map((line) => line.trim())
		.filter(Boolean);

	if (lines.length < 6 || hasStructuredMarkdown(reasoning)) {
		return false;
	}

	const shortLineCount = lines.filter((line) => {
		const wordCount = line.split(/\s+/).filter(Boolean).length;
		return wordCount <= 3 && line.length <= 24;
	}).length;

	return shortLineCount / lines.length >= 0.7;
}

function normalizeReasoningWhitespace(reasoning: string): string {
	let normalized = "";
	let pendingSpace = false;

	for (const ch of reasoning) {
		if (/\s/.test(ch)) {
			pendingSpace = normalized.length > 0;
			continue;
		}

		if (pendingSpace && !/[.,;:!?)}\]'\"]/.test(ch)) {
			normalized += " ";
		}

		pendingSpace = false;
		normalized += ch;
	}

	return normalized;
}

function prepareReasoningForDisplay(reasoning: string): string {
	return looksLikeTokenizedReasoning(reasoning)
		? normalizeReasoningWhitespace(reasoning)
		: reasoning;
}

function shouldRenderAsPlainText(reasoning: string): boolean {
	return looksLikeTokenizedReasoning(reasoning);
}

export function ReasoningViewer({
	reasoning,
	defaultExpanded = false,
	compact = false,
}: ReasoningViewerProps) {
	const [isExpanded, setIsExpanded] = useState(defaultExpanded);
	const [shouldRender, setShouldRender] = useState(defaultExpanded);

	const displayReasoning = useMemo(
		() => prepareReasoningForDisplay(reasoning),
		[reasoning],
	);
	const renderPlainText = useMemo(
		() => shouldRenderAsPlainText(reasoning),
		[reasoning],
	);

	// Lazy render on first expansion
	const handleExpand = () => {
		if (!isExpanded && !shouldRender) {
			setShouldRender(true);
		}
		setIsExpanded(!isExpanded);
	};

	if (!displayReasoning || displayReasoning.trim() === "") {
		return null;
	}

	return (
		<div
			className={
				compact
					? "rounded-lg overflow-hidden bg-muted/20"
					: "mt-2 border rounded-lg overflow-hidden bg-muted/30"
			}
		>
			<button
				onClick={handleExpand}
				className={
					compact
						? "w-full flex items-center justify-between p-2 hover:bg-muted/30 transition-colors"
						: "w-full flex items-center justify-between p-3 hover:bg-muted/50 transition-colors"
				}
			>
				<div className="flex items-center gap-2">
					<BrainCircuit
						className={
							compact ? "w-3 h-3 text-primary" : "w-4 h-4 text-primary"
						}
					/>
					<span
						className={compact ? "text-xs font-medium" : "text-sm font-medium"}
					>
						Reasoning
					</span>
				</div>
				<Button
					variant="ghost"
					size="sm"
					className="h-6 w-6 p-0"
					onClick={(e) => {
						e.stopPropagation();
						handleExpand();
					}}
				>
					{isExpanded ? (
						<ChevronUp className="w-4 h-4" />
					) : (
						<ChevronDown className="w-4 h-4" />
					)}
				</Button>
			</button>

			{isExpanded && shouldRender && (
				<div className={compact ? "" : "border-t"}>
					<div
						className={
							compact
								? "max-h-50 overflow-y-auto scroll-smooth"
								: "max-h-75 overflow-y-auto scroll-smooth"
						}
						style={{
							containIntrinsicSize: compact ? "200px" : "300px",
							contentVisibility: "auto",
						}}
					>
						<div className={compact ? "p-2 text-xs" : "p-3 text-sm"}>
							<Suspense
								fallback={
									<div className="flex items-center justify-center py-4 text-muted-foreground">
										<div className="animate-pulse">Loading...</div>
									</div>
								}
							>
								{renderPlainText ? (
									<div className="whitespace-normal wrap-break-word leading-relaxed text-foreground/90">
										{displayReasoning}
									</div>
								) : (
									<TextEditor
										initialContent={displayReasoning}
										isMarkdown={true}
										editable={false}
										minimal={true}
									/>
								)}
							</Suspense>
						</div>
					</div>
				</div>
			)}
		</div>
	);
}
