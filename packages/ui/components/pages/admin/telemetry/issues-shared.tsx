"use client";

import { Badge } from "../../../ui";
import type { ITelemetryIssueStatus } from "./types";

export const ISSUE_HOUR_OPTIONS: { value: number; label: string }[] = [
	{ value: 24, label: "Last 24 hours" },
	{ value: 72, label: "Last 3 days" },
	{ value: 168, label: "Last 7 days" },
	{ value: 720, label: "Last 30 days" },
	{ value: 2160, label: "Last 90 days" },
];

export const ISSUE_SOURCE_OPTIONS = [
	"desktop",
	"web",
	"web_server",
	"desktop_native",
	"backend",
] as const;

export const ISSUE_LEVEL_OPTIONS = ["fatal", "error", "warning"] as const;

export const ISSUE_STATUS_OPTIONS: ITelemetryIssueStatus[] = [
	"unresolved",
	"resolved",
	"ignored",
];

export function issueLevelTone(level: string) {
	if (level === "fatal") {
		return {
			variant: "destructive" as const,
			ring: "border-destructive/40 bg-destructive/5",
			color: "text-destructive",
		};
	}
	if (level === "warning") {
		return {
			variant: "secondary" as const,
			ring: "border-amber-500/40 bg-amber-500/5",
			color: "text-amber-600 dark:text-amber-400",
		};
	}
	return {
		variant: "outline" as const,
		ring: "border-border bg-muted/30",
		color: "text-muted-foreground",
	};
}

export function issueStatusVariant(status: string) {
	if (status === "resolved") return "secondary" as const;
	if (status === "ignored") return "outline" as const;
	return "default" as const;
}

export function IssueLevelBadge({ level }: { readonly level: string }) {
	return (
		<Badge
			variant={issueLevelTone(level).variant}
			className="font-mono text-[10px] uppercase"
		>
			{level}
		</Badge>
	);
}

export function IssueStatusBadge({ status }: { readonly status: string }) {
	return (
		<Badge variant={issueStatusVariant(status)} className="text-[10px]">
			{status}
		</Badge>
	);
}

export function frameLocation(file?: string | null, lineno?: number | null) {
	if (!file) return "<unknown>";
	return lineno == null ? file : `${file}:${lineno}`;
}

export function safeStringify(value: unknown) {
	try {
		return JSON.stringify(value, null, 2);
	} catch {
		return String(value);
	}
}
