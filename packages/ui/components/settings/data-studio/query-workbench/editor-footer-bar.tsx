"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { Boxes, Cloud, Database, Timer } from "lucide-react";
import { cn } from "../../../../lib/utils";
import type { QuerySurface } from "../../../../state/backend-state/query-state";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";

export interface LastRun {
	durationMs: number;
	rowCount: number;
	ok: boolean;
}

const LIMIT_OPTIONS = [
	{ value: "100", label: "100 rows" },
	{ value: "500", label: "500 rows" },
	{ value: "1000", label: "1,000 rows" },
	{ value: "5000", label: "5,000 rows" },
	{ value: "10000", label: "10,000 rows" },
	{ value: "none", label: "No limit" },
];

function formatDuration(ms: number): string {
	if (ms < 1000) return i18next.t('valMs', '{{val}} ms', { val: Math.round(ms) });
	return `${(ms / 1000).toFixed(2)} s`;
}

export function EditorFooterBar({
	surface,
	overlayName,
	params,
	tables,
	cursor,
	limit,
	onLimitChange,
	lastRun,
	onParamClick,
}: Readonly<{
	surface: QuerySurface | "remote";
	overlayName?: string;
	params: string[];
	tables: string[];
	cursor?: { line: number; column: number };
	limit: number | null;
	onLimitChange: (limit: number | null) => void;
	lastRun: LastRun | null;
	onParamClick?: () => void;
}>) {
	const { t } = useTranslation("settings");
	const SurfaceIcon =
		surface === "remote" ? Cloud : surface === "overlay" ? Boxes : Database;
	const surfaceLabel =
		surface === "remote"
			? (overlayName ?? "Remote ontology")
			: surface === "overlay"
				? (overlayName ?? "Ontology")
				: "Native";
	return (
		<div className="flex h-7 shrink-0 items-center gap-3 overflow-hidden border-t bg-muted/30 px-3 text-[11px] text-muted-foreground">
			<span className="flex shrink-0 items-center gap-1">
				<SurfaceIcon className="h-3 w-3" />
				{surfaceLabel}
			</span>

			{params.length > 0 && (
				<span className="flex shrink-0 items-center gap-1">
					{params.map((name) => (
						<button
							key={name}
							type="button"
							onClick={onParamClick}
							className="rounded bg-muted px-1 font-mono text-[10px] text-foreground/80 hover:bg-muted-foreground/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
						>{`$${name}`}</button>
					))}
				</span>
			)}

			{tables.length > 0 && (
				<span className="hidden min-w-0 items-center gap-1 truncate md:flex">
					<span className="shrink-0">{t('tables', 'tables:')}</span>
					<span className="truncate font-mono">{tables.join(", ")}</span>
				</span>
			)}

			<span className="ml-auto flex shrink-0 items-center gap-3">
				{cursor && (
					<span className="hidden tabular-nums sm:inline">{t('lnLineColColumn', 'Ln {{line}}, Col {{column}}', { line: cursor.line, column: cursor.column })}</span>
				)}

				<Select
					value={limit === null ? "none" : String(limit)}
					onValueChange={(value) =>
						onLimitChange(value === "none" ? null : Number(value))
					}
				>
					<SelectTrigger
						className="h-6 gap-1 border-none bg-transparent px-1.5 text-[11px] shadow-none hover:bg-muted focus:ring-0"
						aria-label={t('rowLimit', 'Row limit')}
					>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{LIMIT_OPTIONS.map((option) => (
							<SelectItem key={option.value} value={option.value}>
								{option.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>

				{lastRun && (
					<span
						className={cn(
							"flex items-center gap-1 tabular-nums",
							lastRun.ok ? "text-muted-foreground" : "text-destructive",
						)}
					>
						<Timer className="h-3 w-3" />
						{formatDuration(lastRun.durationMs)}
						{lastRun.ok && (
							<span>· {lastRun.rowCount.toLocaleString()} rows</span>
						)}
					</span>
				)}
			</span>
		</div>
	);
}
