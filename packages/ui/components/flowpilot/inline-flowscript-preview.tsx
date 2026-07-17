"use client";

import { FileCode2Icon, Loader2 } from "lucide-react";
import { memo } from "react";
import { cn } from "../../lib/utils";
import { isFlowScriptWorkspaceApplicable } from "./flowscript-workspace-candidates";

export interface InlineFlowScriptPreviewValue {
	source: string;
	status?: string;
}

export function resolveLiveFlowScriptPreviewForMessage({
	isLatestMessage,
	messageRole,
	preview,
	workspaceStatus,
}: {
	isLatestMessage: boolean;
	messageRole: string;
	preview: InlineFlowScriptPreviewValue | null | undefined;
	workspaceStatus?: string;
}): InlineFlowScriptPreviewValue | undefined {
	if (!isLatestMessage || messageRole !== "assistant" || !preview) {
		return undefined;
	}
	return {
		...preview,
		status: workspaceStatus ?? preview.status,
	};
}

export function resolveDisplayedFlowScriptPreview({
	messageRole,
	livePreview,
	messageWorkspace,
}: {
	messageRole: string;
	livePreview?: InlineFlowScriptPreviewValue;
	messageWorkspace?: string;
}): InlineFlowScriptPreviewValue | undefined {
	if (messageRole === "user") return undefined;
	return (
		livePreview ?? (messageWorkspace ? { source: messageWorkspace } : undefined)
	);
}

export function isDraftingFlowScriptWorkspace(status?: string): boolean {
	return status === "drafting";
}

export function flowScriptWorkspaceOwnsApply(
	source: string,
	status?: string,
): boolean {
	return isFlowScriptWorkspaceApplicable({ source, status });
}

function formatPreviewLineCount(source: string): string {
	const lines = source ? source.split("\n").length : 0;
	return `${lines} line${lines === 1 ? "" : "s"}`;
}

export const InlineFlowScriptPreview = memo(function InlineFlowScriptPreview({
	preview,
}: {
	preview: InlineFlowScriptPreviewValue;
}) {
	const drafting = isDraftingFlowScriptWorkspace(preview.status);
	const statusLabel = drafting
		? "Writing"
		: preview.status
			? preview.status.replaceAll("_", " ")
			: "FlowScript";

	return (
		<div className="mt-3 min-w-0 max-w-full overflow-hidden rounded-lg border border-border/45 bg-muted/20">
			<div className="flex min-w-0 items-center justify-between gap-2 border-b border-border/35 bg-background/70 px-2.5 py-1.5">
				<div className="flex min-w-0 items-center gap-1.5">
					<FileCode2Icon className="h-3.5 w-3.5 shrink-0 text-primary" />
					<span className="truncate text-[10px] font-semibold uppercase tracking-wide text-foreground">
						FlowScript
					</span>
					{drafting && (
						<Loader2 className="h-3 w-3 shrink-0 animate-spin text-primary" />
					)}
				</div>
				<div className="flex shrink-0 items-center gap-1.5 text-[9px] text-muted-foreground">
					<span>{formatPreviewLineCount(preview.source)}</span>
					<span
						className={cn(
							"rounded-full border px-1.5 py-0.5 font-medium capitalize",
							drafting
								? "border-primary/25 bg-primary/10 text-primary"
								: preview.status === "validation_errors" ||
										preview.status === "interrupted"
									? "border-destructive/25 bg-destructive/10 text-destructive"
									: preview.status === "queued" || preview.status === "applied"
										? "border-green-500/25 bg-green-500/10 text-green-600"
										: "border-border/50 bg-muted/40",
						)}
						aria-live="polite"
					>
						{statusLabel}
					</span>
				</div>
			</div>
			<pre
				className="max-h-72 min-w-0 max-w-full overflow-auto whitespace-pre p-3 font-mono text-[10px] leading-[1.55] text-foreground"
				aria-label="Generated FlowScript source preview"
			>
				<code>{preview.source}</code>
			</pre>
		</div>
	);
});
