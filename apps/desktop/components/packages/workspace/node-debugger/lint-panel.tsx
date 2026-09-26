"use client";

import { Badge, ScrollArea, cn } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	AlertTriangle,
	CheckCircle2,
	Info,
	ShieldCheck,
} from "lucide-react";
import { useMemo, useState } from "react";
import type { LintIssue, LintSeverity } from "../../../../lib/validate-nodes";

function SeverityIcon({ severity }: { severity: LintSeverity }) {
	switch (severity) {
		case "error":
			return <AlertCircle className="h-3.5 w-3.5 text-destructive shrink-0" />;
		case "warning":
			return <AlertTriangle className="h-3.5 w-3.5 text-amber-500 shrink-0" />;
		case "info":
			return <Info className="h-3.5 w-3.5 text-blue-500 shrink-0" />;
	}
}

export function LintPanel({
	issues,
	counts,
	onJumpToNode,
}: {
	issues: LintIssue[];
	counts: { errors: number; warnings: number; infos: number };
	onJumpToNode: (nodeIndex: number) => void;
}) {
	const { t } = useTranslation("common");
	const [filter, setFilter] = useState<LintSeverity | "all">("all");

	const filtered = useMemo(
		() =>
			filter === "all" ? issues : issues.filter((i) => i.severity === filter),
		[issues, filter],
	);

	const total = counts.errors + counts.warnings + counts.infos;

	return (
		<div className="rounded-xl border border-border/20 bg-card/50 p-4 space-y-4">
			<div className="flex flex-wrap items-center justify-between gap-2">
				<div className="flex items-center gap-2">
					<ShieldCheck className="h-3.5 w-3.5 text-muted-foreground/60" />
					<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
						{t("nodeLint", "Node Lint")}
					</span>
				</div>
				<div className="flex flex-wrap items-center gap-1.5">
					{counts.errors > 0 && (
						<Badge
							variant={filter === "error" ? "destructive" : "outline"}
							className="text-[10px] cursor-pointer gap-1"
							onClick={() => setFilter(filter === "error" ? "all" : "error")}
						>
							<AlertCircle className="h-3 w-3" />
							{t("errorsError", "{{errors}} error", { errors: counts.errors })}
							{counts.errors !== 1 ? "s" : ""}
						</Badge>
					)}
					{counts.warnings > 0 && (
						<Badge
							variant="outline"
							className={cn(
								"text-[10px] cursor-pointer gap-1",
								filter === "warning"
									? "bg-amber-500/20 text-amber-600 border-amber-500/40"
									: "text-amber-600 border-amber-500/20",
							)}
							onClick={() =>
								setFilter(filter === "warning" ? "all" : "warning")
							}
						>
							<AlertTriangle className="h-3 w-3" />
							{t("countWarnings", {
								defaultValue_one: "{{count}} warning",
								defaultValue_other: "{{count}} warnings",
								count: counts.warnings,
							})}
						</Badge>
					)}
					{counts.infos > 0 && (
						<Badge
							variant="outline"
							className={cn(
								"text-[10px] cursor-pointer gap-1",
								filter === "info"
									? "bg-blue-500/20 text-blue-600 border-blue-500/40"
									: "text-blue-600 border-blue-500/20",
							)}
							onClick={() => setFilter(filter === "info" ? "all" : "info")}
						>
							<Info className="h-3 w-3" />
							{t("infosInfo", "{{infos}} info", { infos: counts.infos })}
							{counts.infos !== 1 ? "s" : ""}
						</Badge>
					)}
					{total === 0 && (
						<Badge
							variant="outline"
							className="text-[10px] gap-1 text-green-600 border-green-500/20"
						>
							<CheckCircle2 className="h-3 w-3" />
							{t("allClear", "All clear")}
						</Badge>
					)}
				</div>
			</div>

			{filtered.length === 0 ? (
				<div className="text-center py-8">
					<CheckCircle2 className="h-8 w-8 text-green-500/30 mx-auto mb-2" />
					<p className="text-sm text-muted-foreground/60">
						{t("noIssuesMatchingFilter", {
							defaultValue_zero: "No issues found",
							defaultValue_other: "No issues matching filter",
							count: total,
						})}
					</p>
				</div>
			) : (
				<ScrollArea className="max-h-125">
					<div className="space-y-2 pr-3">
						{filtered.map((issue, i) => (
							<button
								key={`${issue.nodeIndex}-${issue.severity}-${i}`}
								type="button"
								className={cn(
									"w-full text-left rounded-lg border p-3 transition-colors hover:bg-muted/10",
									issue.severity === "error"
										? "border-destructive/20 bg-destructive/5"
										: issue.severity === "warning"
											? "border-amber-500/20 bg-amber-500/5"
											: "border-blue-500/20 bg-blue-500/5",
								)}
								onClick={() => onJumpToNode(issue.nodeIndex)}
							>
								<div className="flex items-start gap-2">
									<SeverityIcon severity={issue.severity} />
									<div className="min-w-0 flex-1">
										<div className="flex items-center gap-2 mb-0.5">
											<span className="text-xs font-medium">
												{issue.nodeName}
											</span>
											{issue.pinName && (
												<Badge
													variant="outline"
													className="text-[10px] font-mono"
												>
													{issue.pinName}
												</Badge>
											)}
										</div>
										<p className="text-xs text-muted-foreground/70">
											{issue.message}
										</p>
									</div>
								</div>
							</button>
						))}
					</div>
				</ScrollArea>
			)}
		</div>
	);
}
