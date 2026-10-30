"use client";

import AppWindow from "lucide-react/dist/esm/icons/app-window.js";
import ChevronDown from "lucide-react/dist/esm/icons/chevron-down.js";
import Database from "lucide-react/dist/esm/icons/database.js";
import ExternalLink from "lucide-react/dist/esm/icons/external-link.js";
import FileText from "lucide-react/dist/esm/icons/file-text.js";
import Globe from "lucide-react/dist/esm/icons/globe.js";
import Search from "lucide-react/dist/esm/icons/search.js";
import Workflow from "lucide-react/dist/esm/icons/workflow.js";
import Wrench from "lucide-react/dist/esm/icons/wrench.js";
import { type ReactNode, useId, useState } from "react";
import { cn } from "../../../lib/utils";
import { getToolActivityLabel } from "../../../state/global-chat/tool-step-activity";
import type { IPlanStep, IToolActivityDetail } from "./chat-db";
import { ReasoningBody } from "./reasoning-viewer";

export type ActivitySource = NonNullable<
	IToolActivityDetail["sources"]
>[number];

const ACTIVITY_ICONS = {
	web: Globe,
	search: Search,
	apps: AppWindow,
	files: FileText,
	database: Database,
	workflow: Workflow,
	tool: Wrench,
};

export function stepActivityTitle(step: IPlanStep): string {
	const title = step.title.replace(/^(?:↳\s*)+/, "");
	const isToolLabel =
		title === `Using ${step.toolName}` ||
		(step.detail?.kind === "tool_activity" &&
			/^Using [\w./:-]+$/.test(title)) ||
		title === step.toolName ||
		title === "Step" ||
		title === "Working";
	if (step.title && !isToolLabel) return step.title;
	if (step.detail?.kind === "tool_activity") return step.detail.label;
	if (step.toolName) {
		return getToolActivityLabel(step.toolName);
	}
	return step.title;
}

/** Use only a public hostname for the icon service, never the request path or query. */
function faviconUrl(hostname: string): string | undefined {
	const host = hostname.toLowerCase();
	if (
		!host.includes(".") ||
		/^[\d.]+$/.test(host) ||
		host.includes(":") ||
		/(?:^|\.)(?:localhost|local|internal|test|invalid|example|lan|home|onion)$/.test(
			host,
		)
	) {
		return undefined;
	}
	return `https://www.google.com/s2/favicons?domain=${encodeURIComponent(host)}&sz=32`;
}

export function WebsiteIcon({ source }: { source: ActivitySource }) {
	const url = faviconUrl(source.hostname);
	const [failedUrl, setFailedUrl] = useState<string>();
	return url && failedUrl !== url ? (
		<img
			src={url}
			alt=""
			width={16}
			height={16}
			loading="lazy"
			referrerPolicy="no-referrer"
			onError={() => setFailedUrl(url)}
			className="size-4 shrink-0 rounded-sm object-contain"
		/>
	) : (
		<Globe
			className="size-4 shrink-0 text-muted-foreground"
			aria-hidden="true"
		/>
	);
}

const CHIP_CLASS =
	"flex w-fit min-w-0 max-w-full items-center gap-1.5 rounded-md border border-border/60 bg-background/50 px-2 py-1 text-[11px] text-muted-foreground";

function ContextChip({ text, isQuery }: { text: string; isQuery: boolean }) {
	return (
		<span className={CHIP_CLASS} title={text}>
			{isQuery && (
				<Search className="size-3 shrink-0 opacity-70" aria-hidden="true" />
			)}
			<span className="min-w-0 truncate font-medium text-foreground/80">
				{text}
			</span>
		</span>
	);
}

function SourceChip({ source }: { source: ActivitySource }) {
	const className = cn("group/source", CHIP_CLASS);
	const content = (
		<>
			<WebsiteIcon source={source} />
			<span className="min-w-0 truncate">
				<span className="font-medium text-foreground/80">
					{source.hostname}
				</span>
				{source.path && source.path !== "/" && (
					<span className="ml-1.5">{source.path}</span>
				)}
			</span>
		</>
	);
	if (source.linkable === false) {
		return (
			<span className={className} title={source.url}>
				{content}
			</span>
		);
	}
	return (
		<a
			href={source.url}
			target="_blank"
			rel="noopener noreferrer"
			title={source.url}
			className={cn(
				className,
				"transition-colors hover:bg-muted/50 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40",
			)}
		>
			{content}
			<ExternalLink
				className="size-3 shrink-0 opacity-50 group-hover/source:opacity-100"
				aria-hidden="true"
			/>
		</a>
	);
}

/** Keep the sites visible even after the completed activity group folds away. */
export function ActivitySources({ steps }: { steps: IPlanStep[] }) {
	const sources = new Map<string, ActivitySource>();
	for (const step of steps) {
		if (step.detail?.kind !== "tool_activity") continue;
		for (const source of step.detail.sources ?? []) {
			if (!sources.has(source.hostname)) sources.set(source.hostname, source);
		}
	}
	if (!sources.size) return null;
	return (
		<span className="flex min-w-0 flex-wrap items-center gap-1.5 text-[11px] font-normal text-muted-foreground">
			{[...sources.values()].map((source) => (
				<span
					key={source.hostname}
					title={source.hostname}
					className="inline-flex size-6 items-center justify-center rounded-full border border-border/60 bg-background"
				>
					<WebsiteIcon source={source} />
					<span className="sr-only">{source.hostname}</span>
				</span>
			))}
			<span className="ml-1 tabular-nums">
				{sources.size} {sources.size === 1 ? "website" : "websites"}
			</span>
		</span>
	);
}

/** The outcome supersedes the in-flight progress note, so the meta line stays one line. */
function activityNote(
	detail: IToolActivityDetail,
	description: string | undefined,
	shown: (string | undefined)[],
) {
	if (detail.outcome) return detail.outcome;
	return description && !shown.includes(description) ? description : undefined;
}

export function ToolActivityRow({
	step,
	detail,
	isActive,
	isExpanded,
	onToggle,
	statusIcon,
	duration,
}: {
	step: IPlanStep;
	detail: IToolActivityDetail;
	isActive: boolean;
	isExpanded: boolean;
	onToggle: (stepId: string) => void;
	statusIcon: ReactNode;
	duration: string | null;
}) {
	const Icon = ACTIVITY_ICONS[detail.category] ?? Wrench;
	const reasoningId = useId();
	const hasReasoning = Boolean(step.reasoning?.trim());
	const context = detail.query ?? detail.target;
	const title = stepActivityTitle(step);
	const note = activityNote(detail, step.description?.trim(), [
		context,
		title,
		detail.label,
	]);
	const hasChips = Boolean(context || detail.sources?.length);
	const hasMeta = Boolean(detail.method || note || hasReasoning);
	return (
		<div
			className={cn(
				"min-w-0 rounded-lg px-2 py-1.5 motion-safe:animate-in motion-safe:fade-in motion-safe:duration-300",
				isActive && "bg-primary/5",
			)}
			data-tool-activity={detail.category}
		>
			<div className="flex min-w-0 items-start gap-2.5">
				<span
					className="flex size-7 shrink-0 items-center justify-center rounded-md border border-border/60 bg-muted/30"
					aria-hidden="true"
				>
					<Icon className="size-3.5 text-muted-foreground" />
				</span>
				<div className="min-w-0 flex-1 space-y-1">
					<div className="flex min-h-7 min-w-0 items-center gap-2 text-xs">
						<span
							className={cn(
								"min-w-0 flex-1 truncate font-medium",
								step.status === "failed"
									? "text-muted-foreground"
									: "text-foreground/90",
							)}
							title={title}
						>
							{title}
						</span>
						{duration && (
							<span className="shrink-0 text-[10px] tabular-nums text-muted-foreground/70">
								{duration}
							</span>
						)}
						<span className="shrink-0">{statusIcon}</span>
					</div>
					{hasChips && (
						<div className="flex min-w-0 flex-wrap gap-1.5">
							{context && (
								<ContextChip
									text={context}
									isQuery={context === detail.query}
								/>
							)}
							{detail.sources?.map((source) => (
								<SourceChip key={source.url} source={source} />
							))}
						</div>
					)}
					{hasMeta && (
						<div className="flex min-w-0 items-center gap-2 pt-0.5 text-[11px] text-muted-foreground/80">
							{detail.method && (
								<span className="shrink-0 rounded border border-border/60 px-1 font-mono text-[10px]">
									{detail.method}
								</span>
							)}
							{note && (
								<span className="min-w-0 truncate" title={note}>
									{note}
								</span>
							)}
							{hasReasoning && (
								<button
									type="button"
									onClick={() => onToggle(step.id)}
									aria-expanded={isExpanded}
									aria-controls={reasoningId}
									className="ml-auto flex shrink-0 items-center gap-1 rounded text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
								>
									Details
									<ChevronDown
										className={cn(
											"size-3 transition-transform",
											isExpanded && "rotate-180",
										)}
										aria-hidden="true"
									/>
								</button>
							)}
						</div>
					)}
					{isExpanded && hasReasoning && (
						<div id={reasoningId}>
							<ReasoningBody reasoning={step.reasoning ?? ""} compact />
						</div>
					)}
				</div>
			</div>
		</div>
	);
}
