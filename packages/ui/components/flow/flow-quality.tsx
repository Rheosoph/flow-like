"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BrainIcon,
	ChevronRightIcon,
	CircleCheckIcon,
	CircleXIcon,
	CornerDownLeftIcon,
	CreditCardIcon,
	EyeIcon,
	EyeOffIcon,
	GhostIcon,
	InfinityIcon,
	InfoIcon,
	KeyRoundIcon,
	type LucideIcon,
	ShieldAlertIcon,
	TriangleAlertIcon,
	UnplugIcon,
} from "lucide-react";
import { memo, useCallback, useMemo, useState } from "react";
import {
	type IQualityFinding,
	type IQualityRule,
	type IQualitySeverity,
	QUALITY_RULES,
	describeQualityFinding,
	qualityRuleLabel,
} from "../../lib/board-quality";
import type { IBoard } from "../../lib/schema/flow/board";
import { cn } from "../../lib/utils";
import {
	selectQualityReport,
	useBoardQualityStore,
} from "../../state/board-quality-state";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

const RULE_ICONS: Record<IQualityRule, LucideIcon> = {
	"hardcoded-secret": KeyRoundIcon,
	"payment-readiness": CreditCardIcon,
	cycle: InfinityIcon,
	"no-return": CornerDownLeftIcon,
	"missing-input": UnplugIcon,
	"insecure-node": ShieldAlertIcon,
	"dead-code": GhostIcon,
	complexity: BrainIcon,
};

const SEVERITY_ICONS: Record<IQualitySeverity, LucideIcon> = {
	error: CircleXIcon,
	warning: TriangleAlertIcon,
	info: InfoIcon,
};

const SEVERITY_TEXT: Record<IQualitySeverity, string> = {
	error: "text-destructive",
	warning: "text-amber-500",
	info: "text-sky-500",
};

const SEVERITY_ORDER: readonly IQualitySeverity[] = [
	"error",
	"warning",
	"info",
];

export const QualitySeverityIcon = memo(function QualitySeverityIcon({
	severity,
	className,
}: Readonly<{ severity: IQualitySeverity; className?: string }>) {
	const Icon = SEVERITY_ICONS[severity];
	return <Icon className={cn(SEVERITY_TEXT[severity], className)} />;
});

/** Worst-scored nodes lead the insecure group; every other group keeps the report order. */
function orderGroup(rule: IQualityRule, findings: IQualityFinding[]) {
	if (rule !== "insecure-node") return findings;
	return findings.slice().sort((a, b) => {
		const scoreA = a.detail.kind === "low-score" ? a.detail.score : 11;
		const scoreB = b.detail.kind === "low-score" ? b.detail.score : 11;
		return scoreA - scoreB;
	});
}

const SeverityChip = memo(function SeverityChip({
	severity,
	count,
	active,
	onToggle,
}: Readonly<{
	severity: IQualitySeverity;
	count: number;
	active: boolean;
	onToggle: () => void;
}>) {
	const { t } = useTranslation("flow");
	const labels: Record<IQualitySeverity, string> = {
		error: t("qualityErrors", "Errors"),
		warning: t("qualityWarnings", "Warnings"),
		info: t("qualityHints", "Hints"),
	};
	return (
		<button
			type="button"
			aria-pressed={active}
			title={labels[severity]}
			onClick={onToggle}
			className={cn(
				"flex h-5 items-center gap-1 rounded-sm px-1.5 text-[11px] tabular-nums transition-colors",
				active
					? "bg-accent text-foreground"
					: "text-muted-foreground/60 hover:text-muted-foreground",
			)}
		>
			<QualitySeverityIcon
				severity={severity}
				className={cn("size-3", !active && "opacity-50")}
			/>
			{count}
		</button>
	);
});

const FindingRow = memo(function FindingRow({
	finding,
	layerName,
	onSelect,
}: Readonly<{
	finding: IQualityFinding;
	layerName?: string;
	onSelect: (finding: IQualityFinding) => void;
}>) {
	const { t } = useTranslation("flow");
	const description = describeQualityFinding(finding, t);
	return (
		<li>
			<button
				type="button"
				onClick={() => onSelect(finding)}
				className="flex w-full items-start gap-2 rounded-sm px-2 py-1 text-left hover:bg-accent"
			>
				<QualitySeverityIcon
					severity={finding.severity}
					className="mt-0.5 size-3 shrink-0"
				/>
				<span className="flex min-w-0 flex-1 flex-col gap-0.5">
					<span className="flex min-w-0 items-baseline gap-1.5 text-xs">
						<span className="truncate font-medium">{finding.target.name}</span>
						{layerName && (
							<span className="shrink-0 truncate text-[10px] text-muted-foreground">
								{layerName}
							</span>
						)}
						{finding.target.kind === "variable" && (
							<span className="shrink-0 text-[10px] text-muted-foreground">
								{t("variable", "Variable")}
							</span>
						)}
					</span>
					<span className="text-[11px] leading-snug text-muted-foreground">
						{description}
					</span>
				</span>
			</button>
		</li>
	);
});

const RuleGroup = memo(function RuleGroup({
	rule,
	findings,
	open,
	onToggle,
	layerNames,
	onSelect,
}: Readonly<{
	rule: IQualityRule;
	findings: IQualityFinding[];
	open: boolean;
	onToggle: (rule: IQualityRule) => void;
	layerNames: ReadonlyMap<string, string>;
	onSelect: (finding: IQualityFinding) => void;
}>) {
	const { t } = useTranslation("flow");
	const Icon = RULE_ICONS[rule];
	const worst =
		SEVERITY_ORDER.find((severity) =>
			findings.some((finding) => finding.severity === severity),
		) ?? "info";
	return (
		<li>
			<button
				type="button"
				aria-expanded={open}
				onClick={() => onToggle(rule)}
				className="sticky top-0 z-10 flex h-7 w-full items-center gap-1.5 border-b bg-background px-1.5 text-left text-xs hover:bg-accent/50"
			>
				<ChevronRightIcon
					className={cn(
						"size-3 shrink-0 text-muted-foreground transition-transform",
						open && "rotate-90",
					)}
				/>
				<Icon className={cn("size-3.5 shrink-0", SEVERITY_TEXT[worst])} />
				<span className="min-w-0 flex-1 truncate font-medium">
					{qualityRuleLabel(rule, t)}
				</span>
				<span className="rounded-full bg-muted px-1.5 text-[10px] tabular-nums text-muted-foreground">
					{findings.length}
				</span>
			</button>
			{open && (
				<ul className="flex flex-col py-0.5">
					{findings.map((finding) => (
						<FindingRow
							key={finding.id}
							finding={finding}
							layerName={
								finding.target.layerId
									? layerNames.get(finding.target.layerId)
									: undefined
							}
							onSelect={onSelect}
						/>
					))}
				</ul>
			)}
		</li>
	);
});

export const FlowQuality = memo(function FlowQuality({
	boardId,
	board,
	onFocusNode,
	onOpenVariables,
}: Readonly<{
	boardId: string;
	board: IBoard | undefined;
	onFocusNode: (id: string) => void;
	/** Findings on a variable have no canvas position; they open the variables view instead. */
	onOpenVariables: () => void;
}>) {
	const { t } = useTranslation("flow");
	const report = useBoardQualityStore(
		useMemo(() => selectQualityReport(boardId), [boardId]),
	);
	const inline = useBoardQualityStore((state) => state.inline);
	const setInline = useBoardQualityStore((state) => state.setInline);
	const filter = useBoardQualityStore((state) => state.filter);
	const toggleSeverity = useBoardQualityStore((state) => state.toggleSeverity);
	const [collapsed, setCollapsed] = useState<
		Partial<Record<IQualityRule, boolean>>
	>({});

	const layerNames = useMemo(() => {
		const names = new Map<string, string>();
		for (const layer of Object.values(board?.layers ?? {})) {
			if (layer?.id) names.set(layer.id, layer.name);
		}
		return names;
	}, [board?.layers]);

	const groups = useMemo(() => {
		const byRule = new Map<IQualityRule, IQualityFinding[]>();
		for (const finding of report.findings) {
			if (!filter[finding.severity]) continue;
			const list = byRule.get(finding.rule) ?? [];
			list.push(finding);
			byRule.set(finding.rule, list);
		}
		return QUALITY_RULES.flatMap((rule) => {
			const findings = byRule.get(rule);
			return findings ? [{ rule, findings: orderGroup(rule, findings) }] : [];
		});
	}, [report.findings, filter]);

	const visible = groups.reduce((sum, group) => sum + group.findings.length, 0);

	const toggleGroup = useCallback((rule: IQualityRule) => {
		setCollapsed((previous) => ({ ...previous, [rule]: !previous[rule] }));
	}, []);

	const select = useCallback(
		(finding: IQualityFinding) => {
			if (finding.target.kind === "variable") {
				onOpenVariables();
				return;
			}
			onFocusNode(finding.target.id);
		},
		[onFocusNode, onOpenVariables],
	);

	return (
		<div className="flex h-full min-h-0 flex-col">
			<div className="flex h-8 shrink-0 items-center gap-0.5 border-b px-1.5">
				{SEVERITY_ORDER.map((severity) => (
					<SeverityChip
						key={severity}
						severity={severity}
						count={report.counts[severity]}
						active={filter[severity]}
						onToggle={() => toggleSeverity(severity)}
					/>
				))}
				<span className="flex-1" />
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							aria-pressed={inline}
							onClick={() => setInline(!inline)}
							className={cn(
								"flex size-6 items-center justify-center rounded-sm transition-colors hover:bg-accent",
								inline ? "text-foreground" : "text-muted-foreground/60",
							)}
						>
							{inline ? (
								<EyeIcon className="size-3.5" />
							) : (
								<EyeOffIcon className="size-3.5" />
							)}
						</button>
					</TooltipTrigger>
					<TooltipContent side="bottom">
						{inline
							? t("qualityHideOnCanvas", "Hide markers on the canvas")
							: t("qualityShowOnCanvas", "Show markers on the canvas")}
					</TooltipContent>
				</Tooltip>
			</div>
			{report.findings.length === 0 ? (
				<div className="flex flex-1 flex-col items-center justify-center gap-1 p-4 text-center">
					<CircleCheckIcon className="size-5 text-emerald-500" />
					<p className="text-sm font-medium">
						{t("qualityNoIssues", "No issues found")}
					</p>
					<p className="text-xs text-muted-foreground">
						{report.nodeCount > 0
							? t(
									"qualityNodesChecked",
									"{{count}} nodes checked. Findings appear here as you edit.",
									{ count: report.nodeCount },
								)
							: t("qualityWaiting", "Findings appear here as you edit.")}
					</p>
				</div>
			) : visible === 0 ? (
				<p className="p-4 text-center text-xs text-muted-foreground">
					{t(
						"qualityAllFiltered",
						"Every finding is hidden by the severity filter.",
					)}
				</p>
			) : (
				<ul className="min-h-0 flex-1 overflow-auto">
					{groups.map((group) => (
						<RuleGroup
							key={group.rule}
							rule={group.rule}
							findings={group.findings}
							open={!collapsed[group.rule]}
							onToggle={toggleGroup}
							layerNames={layerNames}
							onSelect={select}
						/>
					))}
				</ul>
			)}
			<p className="shrink-0 border-t px-2 py-1 text-[10px] text-muted-foreground tabular-nums">
				{t(
					"qualityFooter",
					"{{count}} findings · {{nodes}} nodes · {{ms}} ms",
					{
						count: report.findings.length,
						nodes: report.nodeCount,
						ms: Math.round(report.durationMs),
					},
				)}
			</p>
		</div>
	);
});
