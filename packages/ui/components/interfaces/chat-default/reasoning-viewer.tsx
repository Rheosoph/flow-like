"use client";

import { useTranslation } from "@flow-like/locales";
import BrainCircuit from "lucide-react/dist/esm/icons/brain-circuit.js";
import ChevronDown from "lucide-react/dist/esm/icons/chevron-down.js";
import ChevronUp from "lucide-react/dist/esm/icons/chevron-up.js";
import { Suspense, lazy, useMemo, useState } from "react";
import { cn } from "../../../lib/utils";
import {
	looksLikeTokenizedReasoning,
	sanitizeReasoningForDisplay,
} from "./reasoning-format";

const TextEditor = lazy(() =>
	import("../../ui/text-editor").then((m) => ({ default: m.TextEditor })),
);

interface ReasoningViewerProps {
	reasoning: string;
	defaultExpanded?: boolean;
	compact?: boolean;
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
		() => sanitizeReasoningForDisplay(reasoning),
		[reasoning],
	);
	const renderPlainText = useMemo(
		() => looksLikeTokenizedReasoning(reasoning),
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
						"overflow-y-auto scroll-smooth rounded-lg border border-border/50 bg-muted/25",
						compact ? "mt-1.5 max-h-50 px-3 py-2" : "mt-2 max-h-75 px-3.5 py-2.5",
					)}
					style={{
						containIntrinsicSize: compact ? "200px" : "300px",
						contentVisibility: "auto",
						maxWidth: "var(--fl-chat-measure, 38rem)",
					}}
				>
					<div
						className="text-muted-foreground/90"
						style={{
							fontFamily: "var(--fl-chat-prose-font)",
							fontSize: compact ? "0.8125rem" : "0.85rem",
							lineHeight: 1.7,
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
