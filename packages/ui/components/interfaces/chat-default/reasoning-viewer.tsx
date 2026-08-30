"use client";

import { useTranslation } from "@flow-like/locales";
import BrainCircuit from "lucide-react/dist/esm/icons/brain-circuit.js";
import ChevronDown from "lucide-react/dist/esm/icons/chevron-down.js";
import ChevronUp from "lucide-react/dist/esm/icons/chevron-up.js";
import { Suspense, lazy, useMemo, useState } from "react";
import { cn } from "../../../lib/utils";

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
	const { t } = useTranslation("chat");
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
		<div className={compact ? "" : "mt-2"}>
			<button
				type="button"
				onClick={handleExpand}
				aria-expanded={isExpanded}
				className="flex w-full items-center gap-2 rounded-md py-1 text-left text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
			>
				<BrainCircuit className={compact ? "size-3" : "size-3.5"} />
				<span
					className={cn("font-medium", compact ? "text-[11px]" : "text-xs")}
				>
					{t("reasoning", "Reasoning")}
				</span>
				{isExpanded ? (
					<ChevronUp className="size-3.5" />
				) : (
					<ChevronDown className="size-3.5" />
				)}
			</button>

			{isExpanded && shouldRender && (
				<div
					className={cn(
						"overflow-y-auto scroll-smooth border-l pl-3",
						compact ? "mt-1 ml-1 max-h-50" : "mt-1.5 ml-1 max-h-75",
					)}
					style={{
						borderColor: "var(--fl-chat-rule, var(--border))",
						containIntrinsicSize: compact ? "200px" : "300px",
						contentVisibility: "auto",
						maxWidth: "var(--fl-chat-measure, 38rem)",
					}}
				>
					<div
						className="py-0.5 text-muted-foreground italic"
						style={{
							fontFamily: "var(--fl-chat-prose-font)",
							fontSize: compact ? "0.8125rem" : "0.875rem",
							lineHeight: 1.65,
						}}
					>
						<Suspense
							fallback={
								<div className="flex items-center justify-center py-4 text-muted-foreground">
									<div className="animate-pulse">
										{t("loading", "Loading…")}
									</div>
								</div>
							}
						>
							{renderPlainText ? (
								<div className="whitespace-normal wrap-break-word">
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
			)}
		</div>
	);
}
