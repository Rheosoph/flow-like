"use client";

import { useTranslation } from "@flow-like/locales";
import Bug from "lucide-react/dist/esm/icons/bug.js";
import Check from "lucide-react/dist/esm/icons/check.js";
import ChevronDown from "lucide-react/dist/esm/icons/chevron-down.js";
import ChevronRight from "lucide-react/dist/esm/icons/chevron-right.js";
import Copy from "lucide-react/dist/esm/icons/copy.js";
import FileText from "lucide-react/dist/esm/icons/file-text.js";
import { useMemo, useState } from "react";
import { useDeveloperMode } from "../../../hooks/use-developer-mode";
import { useModelNames } from "../../../hooks/use-model-names";
import { modelLabel } from "../../../lib/bit/model-display-name";
import { cn } from "../../../lib/utils";
import {
	type IAgentDebugReport,
	agentDebugReportAsMarkdown,
} from "../../../state/global-chat/agent-debug-report";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../../ui/collapsible";

function outcomeClass(outcome: IAgentDebugReport["outcome"]) {
	if (outcome === "ok") return "text-emerald-500";
	if (outcome === "running") return "text-primary";
	if (outcome === "partial") return "text-amber-500";
	return "text-red-500";
}

export function AgentDebugReport({ report }: { report: IAgentDebugReport }) {
	const { t, i18n } = useTranslation("chat");
	const locale = i18n.resolvedLanguage ?? i18n.language;
	const durationLabel = (duration?: number) => {
		if (duration === undefined) return t("debugReport.running", "Running");
		if (duration < 1_000)
			return t("debugReport.milliseconds", "{{value}} ms", {
				value: Math.round(duration).toLocaleString(locale),
			});
		const digits = duration < 10_000 ? 1 : 0;
		return t("debugReport.seconds", "{{value}} s", {
			value: (duration / 1_000).toLocaleString(locale, {
				minimumFractionDigits: digits,
				maximumFractionDigits: digits,
			}),
		});
	};
	const outcomes: Record<IAgentDebugReport["outcome"], string> = {
		running: t("debugReport.running", "Running"),
		ok: t("debugReport.ok", "Completed"),
		partial: t("debugReport.partial", "Partially completed"),
		error: t("debugReport.failed", "Failed"),
		cancelled: t("debugReport.cancelled", "Cancelled"),
		timeout: t("debugReport.timeout", "Timed out"),
		interrupted: t("debugReport.interrupted", "Interrupted"),
	};
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
						<span className="font-medium">
							{t("debugReport.title", "Debug report")}
						</span>
						<span
							className={cn(
								"font-medium capitalize",
								outcomeClass(report.outcome),
							)}
						>
							{outcomes[report.outcome]}
						</span>
						<span className="text-muted-foreground">
							{durationLabel(report.duration_ms)} ·{" "}
							{t("debugReport.events", {
								defaultValue_one: "{{count}} event",
								defaultValue_other: "{{count}} events",
								count: report.events.length,
							})}
							{report.truncation?.events_dropped
								? ` · ${t("debugReport.omitted", {
										defaultValue_one: "{{count}} omitted",
										defaultValue_other: "{{count}} omitted",
										count: report.truncation.events_dropped,
									})}`
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
									.join(" · ") ||
									t(
										"debugReport.providerUnavailable",
										"Provider metadata unavailable",
									)}
							</span>
							{report.terminal_code && (
								<span className="font-mono">{report.terminal_code}</span>
							)}
							<div className="ml-auto flex items-center gap-1">
								<button
									type="button"
									onClick={() => void copy("json")}
									className="inline-flex items-center gap-1 rounded px-1.5 py-1 hover:bg-muted"
									title={t("debugReport.copyJson", "Copy JSON report")}
								>
									{copied === "json" ? (
										<Check className="size-3" />
									) : (
										<Copy className="size-3" />
									)}
									JSON
								</button>
								<button
									type="button"
									onClick={() => void copy("markdown")}
									className="inline-flex items-center gap-1 rounded px-1.5 py-1 hover:bg-muted"
									title={t("debugReport.copyMarkdown", "Copy Markdown report")}
								>
									{copied === "markdown" ? (
										<Check className="size-3" />
									) : (
										<FileText className="size-3" />
									)}
									Markdown
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
										<span className="text-muted-foreground">
											{t("debugReport.runInput", "Run input")}
										</span>
										<pre className="mt-0.5 whitespace-pre-wrap break-words">
											{report.input_preview}
										</pre>
									</div>
								)}
								{report.output_preview && (
									<div>
										<span className="text-muted-foreground">
											{t("debugReport.runOutput", "Run output")}
										</span>
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
											{new Date(event.timestamp_ms).toLocaleTimeString(locale)}
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
											{event.request_id &&
												t("debugReport.request", "Request: {{id}}", {
													id: event.request_id,
												})}
											{event.request_id && event.parent_request_id && " · "}
											{event.parent_request_id &&
												t("debugReport.parent", "Parent request: {{id}}", {
													id: event.parent_request_id,
												})}
										</p>
									)}
									{event.terminal_status && (
										<p className="mt-1 break-all text-muted-foreground">
											{t(
												"debugReport.terminalStatus",
												"Terminal status: {{status}}",
												{ status: event.terminal_status },
											)}
										</p>
									)}
									{event.summary && (
										<p className="mt-1 whitespace-pre-wrap text-muted-foreground">
											{event.summary}
										</p>
									)}
									{event.arguments_preview && (
										<div className="mt-1">
											<span className="text-muted-foreground">
												{event.kind === "nested"
													? t("debugReport.nestedInput", "Nested input")
													: t("debugReport.input", "Input")}
											</span>
											<pre className="mt-0.5 whitespace-pre-wrap break-words">
												{event.arguments_preview}
											</pre>
										</div>
									)}
									{event.result_summary && (
										<p className="mt-1 break-all">
											{t("debugReport.result", "Result: {{result}}", {
												result: event.result_summary,
											})}
										</p>
									)}
									{event.result_preview && (
										<div className="mt-1">
											<span className="text-muted-foreground">
												{event.kind === "nested"
													? t("debugReport.nestedOutput", "Nested output")
													: t("debugReport.output", "Output")}
											</span>
											<pre className="mt-0.5 whitespace-pre-wrap break-words text-muted-foreground">
												{event.result_preview}
											</pre>
										</div>
									)}
									{event.reasoning && (
										<p className="mt-1 whitespace-pre-wrap text-muted-foreground">
											{t(
												"debugReport.reasoning",
												"Surfaced reasoning: {{reasoning}}",
												{ reasoning: event.reasoning },
											)}
										</p>
									)}
									{event.error && (
										<p className="mt-1 break-all text-red-500">
											{t("debugReport.error", "Error: {{error}}", {
												error: event.error,
											})}
										</p>
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
