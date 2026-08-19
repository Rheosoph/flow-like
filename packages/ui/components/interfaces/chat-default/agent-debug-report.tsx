"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	Bug,
	Check,
	ChevronDown,
	ChevronRight,
	Copy,
	FileText,
} from "lucide-react";
import { useMemo, useState } from "react";
import { useDeveloperMode } from "../../../hooks/use-developer-mode";
import { useModelNames } from "../../../hooks/use-model-names";
import { cn, modelLabel } from "../../../lib";
import {
	type IAgentDebugReport,
	agentDebugReportAsMarkdown,
} from "../../../state/global-chat/agent-debug-report";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../../ui/collapsible";

function durationLabel(duration?: number) {
	if (duration === undefined) return "running";
	if (duration < 1_000)
		return i18next.t("valMs", "{{val}} ms", { val: Math.round(duration) });
	return `${(duration / 1_000).toFixed(duration < 10_000 ? 1 : 0)} s`;
}

function outcomeClass(outcome: IAgentDebugReport["outcome"]) {
	if (outcome === "ok") return "text-emerald-500";
	if (outcome === "running") return "text-primary";
	if (outcome === "partial") return "text-amber-500";
	return "text-red-500";
}

export function AgentDebugReport({ report }: { report: IAgentDebugReport }) {
	const { t } = useTranslation("chat");
	const { developerMode } = useDeveloperMode();
	const [open, setOpen] = useState(false);
	const [copied, setCopied] = useState<"json" | "markdown" | null>(null);
	const modelNames = useModelNames(
		useMemo(() => [report.model], [report.model]),
	);
	if (!developerMode) return null;
	const modelName = report.model
		? modelLabel(report.model, modelNames).label
		: undefined;
	const copy = async (format: "json" | "markdown") => {
		const value =
			format === "json"
				? JSON.stringify(report, null, 2)
				: agentDebugReportAsMarkdown(report);
		try {
			await navigator.clipboard.writeText(value);
			setCopied(format);
			window.setTimeout(() => setCopied(null), 1_500);
		} catch {
			// Clipboard access can be unavailable in an insecure browser context. The report remains
			// visible and selectable, so a failed convenience action must not affect the message.
		}
	};

	return (
		<Collapsible open={open} onOpenChange={setOpen} className="mt-2 w-full">
			<div className="rounded-md border border-border/60 bg-muted/20 text-xs">
				<CollapsibleTrigger asChild>
					<button
						type="button"
						className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left outline-none transition-colors hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-primary/40"
					>
						<Bug className="size-3.5 text-muted-foreground" />
						<span className="font-medium">{`Debug report`}</span>
						<span
							className={cn(
								"font-medium capitalize",
								outcomeClass(report.outcome),
							)}
						>
							{report.outcome}
						</span>
						<span className="text-muted-foreground">
							{durationLabel(report.duration_ms)} · {report.events.length}{" "}
							events
							{report.truncation?.events_dropped
								? ` · ${report.truncation.events_dropped} omitted`
								: ""}
						</span>
						{open ? (
							<ChevronDown className="ml-auto size-3.5 text-muted-foreground" />
						) : (
							<ChevronRight className="ml-auto size-3.5 text-muted-foreground" />
						)}
					</button>
				</CollapsibleTrigger>
				<CollapsibleContent>
					<div className="border-t border-border/50 px-2.5 py-2">
						<div className="mb-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
							<span title={report.model}>
								{[report.provider, modelName, report.reasoning_effort]
									.filter(Boolean)
									.join(" · ") || `Provider metadata unavailable`}
							</span>
							{report.terminal_code && (
								<span className="font-mono">{report.terminal_code}</span>
							)}
							<div className="ml-auto flex items-center gap-1">
								<button
									type="button"
									onClick={() => void copy("json")}
									className="inline-flex items-center gap-1 rounded px-1.5 py-1 hover:bg-muted"
									title={`Copy JSON report`}
								>
									{copied === "json" ? (
										<Check className="size-3" />
									) : (
										<Copy className="size-3" />
									)}
									{`JSON`}
								</button>
								<button
									type="button"
									onClick={() => void copy("markdown")}
									className="inline-flex items-center gap-1 rounded px-1.5 py-1 hover:bg-muted"
									title={`Copy Markdown report`}
								>
									{copied === "markdown" ? (
										<Check className="size-3" />
									) : (
										<FileText className="size-3" />
									)}
									{`Markdown`}
								</button>
							</div>
						</div>
						{report.summary && (
							<p className="mb-2 text-[11px] text-muted-foreground">
								{report.summary}
							</p>
						)}
						{(report.input_preview || report.output_preview) && (
							<div className="mb-2 space-y-1 rounded bg-background/50 px-2 py-1.5 font-mono text-[10px]">
								{report.input_preview && (
									<div>
										<span className="text-muted-foreground">{`Run input`}</span>
										<pre className="mt-0.5 whitespace-pre-wrap break-words">
											{report.input_preview}
										</pre>
									</div>
								)}
								{report.output_preview && (
									<div>
										<span className="text-muted-foreground">{`Run output`}</span>
										<pre className="mt-0.5 whitespace-pre-wrap break-words text-muted-foreground">
											{report.output_preview}
										</pre>
									</div>
								)}
							</div>
						)}
						<div className="max-h-80 space-y-1 overflow-y-auto font-mono text-[10px]">
							{report.events.map((event) => (
								<div
									key={event.id}
									className="rounded bg-background/70 px-2 py-1.5"
								>
									<div className="flex flex-wrap items-center gap-x-2">
										<span className="text-muted-foreground">
											{new Date(event.timestamp_ms).toLocaleTimeString()}
										</span>
										<span>{event.stage}</span>
										{event.name && (
											<span className="font-semibold">{event.name}</span>
										)}
										{event.status && (
											<span className="text-muted-foreground">
												{event.status}
											</span>
										)}
										{event.duration_ms !== undefined && (
											<span className="ml-auto text-muted-foreground">
												{durationLabel(event.duration_ms)}
											</span>
										)}
									</div>
									{(event.request_id || event.parent_request_id) && (
										<p className="mt-1 break-all text-muted-foreground">
											{event.request_id && `request: ${event.request_id}`}
											{event.request_id && event.parent_request_id && ` · `}
											{event.parent_request_id &&
												`parent: ${event.parent_request_id}`}
										</p>
									)}
									{event.terminal_status && (
										<p className="mt-1 break-all text-muted-foreground">{`terminal status: ${event.terminal_status}`}</p>
									)}
									{event.summary && (
										<p className="mt-1 whitespace-pre-wrap text-muted-foreground">
											{event.summary}
										</p>
									)}
									{event.arguments_preview && (
										<div className="mt-1">
											<span className="text-muted-foreground">
												{event.kind === "nested" ? "Nested input" : "Input"}
											</span>
											<pre className="mt-0.5 whitespace-pre-wrap break-words">
												{event.arguments_preview}
											</pre>
										</div>
									)}
									{event.result_summary && (
										<p className="mt-1 break-all">{`result: ${event.result_summary}`}</p>
									)}
									{event.result_preview && (
										<div className="mt-1">
											<span className="text-muted-foreground">
												{event.kind === "nested" ? "Nested output" : "Output"}
											</span>
											<pre className="mt-0.5 whitespace-pre-wrap break-words text-muted-foreground">
												{event.result_preview}
											</pre>
										</div>
									)}
									{event.reasoning && (
										<p className="mt-1 whitespace-pre-wrap text-muted-foreground">{`surfaced reasoning: ${event.reasoning}`}</p>
									)}
									{event.error && (
										<p className="mt-1 break-all text-red-500">{`error: ${event.error}`}</p>
									)}
								</div>
							))}
						</div>
					</div>
				</CollapsibleContent>
			</div>
		</Collapsible>
	);
}
