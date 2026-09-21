"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ShieldAlert,
	ShieldCheck,
	ShieldEllipsis,
	ShieldQuestion,
} from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../../lib/utils";
import { Alert, AlertDescription, AlertTitle } from "../ui/alert";
import { Badge } from "../ui/badge";
import type { IAuditChainReport, IAuditEpochReport } from "./types";

export type AuditIntegrity =
	| "verified"
	| "anchoring"
	| "empty"
	| "broken"
	| "held"
	| "tamperedPending"
	| "unverifiable"
	| "incomplete";

export function chainIntegrity(report: IAuditChainReport): AuditIntegrity {
	if (report.first_broken_seal != null) return "broken";
	if (report.held) return "held";
	if (report.pending_invalid > 0) return "tamperedPending";
	if (!report.valid) {
		return report.unverifiable_epochs > 0 ? "unverifiable" : "incomplete";
	}
	if (report.empty) return "empty";
	return report.unanchored_seals > 0 ? "anchoring" : "verified";
}

export function epochIntegrity(report: IAuditEpochReport): AuditIntegrity {
	if (report.first_broken_epoch != null) return "broken";
	if (!report.valid) {
		return report.unverifiable_epochs > 0 ? "unverifiable" : "incomplete";
	}
	return report.epochs_checked === 0 && report.latest_epoch_seq == null
		? "empty"
		: "verified";
}

export function isIntegrityFailure(integrity: AuditIntegrity): boolean {
	return (
		integrity === "broken" ||
		integrity === "held" ||
		integrity === "tamperedPending"
	);
}

export function AuditIntegrityBadge({
	integrity,
	className,
}: Readonly<{ integrity: AuditIntegrity; className?: string }>) {
	const { t } = useTranslation("audit");
	const variants: Record<
		AuditIntegrity,
		{
			label: string;
			icon: ReactNode;
			variant: "default" | "secondary" | "destructive" | "outline";
		}
	> = {
		verified: {
			label: t("integrityVerified", "Verified"),
			icon: <ShieldCheck />,
			variant: "default",
		},
		anchoring: {
			label: t("integrityAnchoring", "Intact, anchoring pending"),
			icon: <ShieldEllipsis />,
			variant: "secondary",
		},
		empty: {
			label: t("integrityEmpty", "No records yet"),
			icon: <ShieldQuestion />,
			variant: "outline",
		},
		broken: {
			label: t("integrityBroken", "Broken"),
			icon: <ShieldAlert />,
			variant: "destructive",
		},
		held: {
			label: t("integrityHeld", "Held for review"),
			icon: <ShieldAlert />,
			variant: "destructive",
		},
		tamperedPending: {
			label: t("integrityTamperedPending", "Pending record failed"),
			icon: <ShieldAlert />,
			variant: "destructive",
		},
		unverifiable: {
			label: t("integrityUnverifiable", "Key unavailable"),
			icon: <ShieldQuestion />,
			variant: "outline",
		},
		incomplete: {
			label: t("integrityIncomplete", "Not fully verified"),
			icon: <ShieldEllipsis />,
			variant: "outline",
		},
	};
	const { label, icon, variant } = variants[integrity];
	return (
		<Badge variant={variant} className={cn("gap-1", className)}>
			{icon}
			{label}
		</Badge>
	);
}

function ReportStat({
	label,
	value,
	emphasis = false,
}: Readonly<{ label: string; value: ReactNode; emphasis?: boolean }>) {
	return (
		<div className="rounded-md border bg-muted/30 px-2.5 py-1.5">
			<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
				{label}
			</div>
			<div
				className={cn(
					"font-mono text-sm tabular-nums",
					emphasis && "text-destructive",
				)}
			>
				{value}
			</div>
		</div>
	);
}

function seqOrDash(value?: number | null): string {
	return value == null ? "—" : `#${value}`;
}

function CheckedRange({ report }: Readonly<{ report: IAuditChainReport }>) {
	const { t } = useTranslation("audit");
	const from = report.checked_from_seq;
	if (report.latest_seal_seq == null || from == null) return null;
	if (report.seals_checked === 0) {
		if (from <= 1) return null;
		return (
			<p className="text-xs text-muted-foreground">
				{t(
					"checkedNone",
					"No seals after #{{seq}} to check. Earlier seals were verified by a previous check on this server, or pruned.",
					{ seq: from - 1 },
				)}
			</p>
		);
	}
	return (
		<p className="text-xs text-muted-foreground">
			{t("checkedRange", "Checked seals #{{from}}–#{{to}}.", {
				from,
				to: from + report.seals_checked - 1,
			})}
			{from > 1 &&
				` ${t(
					"checkedEarlier",
					"Seals before #{{seq}} were verified by a previous check on this server, or pruned.",
					{ seq: from },
				)}`}
		</p>
	);
}

export function AuditChainReportView({
	report,
}: Readonly<{ report: IAuditChainReport }>) {
	const { t } = useTranslation("audit");
	const integrity = chainIntegrity(report);
	return (
		<div className="space-y-3">
			<AuditIntegrityBadge integrity={integrity} />
			<CheckedRange report={report} />
			{report.held && (
				<Alert variant="destructive">
					<ShieldAlert className="h-4 w-4" />
					<AlertTitle>{t("reportHeldTitle", "Chain held")}</AlertTitle>
					<AlertDescription>
						{t(
							"reportHeld",
							"A seal of this chain failed its hash or MAC check. The chain is not signed into an epoch until an operator investigates.",
						)}
					</AlertDescription>
				</Alert>
			)}
			{report.problem && (
				<Alert
					variant={isIntegrityFailure(integrity) ? "destructive" : "default"}
				>
					<ShieldAlert className="h-4 w-4" />
					<AlertTitle>
						{report.first_broken_seal != null
							? t("reportBrokenAt", "First failing seal: #{{seq}}", {
									seq: report.first_broken_seal,
								})
							: t("reportProblem", "Verification did not complete")}
					</AlertTitle>
					<AlertDescription className="font-mono text-xs">
						{report.problem}
					</AlertDescription>
				</Alert>
			)}
			<div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
				<ReportStat
					label={t("sealsChecked", "Seals checked")}
					value={report.seals_checked.toLocaleString()}
				/>
				<ReportStat
					label={t("recordsChecked", "Records checked")}
					value={report.records_checked.toLocaleString()}
				/>
				<ReportStat
					label={t("pendingRecords", "Pending records")}
					value={report.pending_records.toLocaleString()}
				/>
				<ReportStat
					label={t("pendingInvalid", "Failed pending checks")}
					value={report.pending_invalid.toLocaleString()}
					emphasis={report.pending_invalid > 0}
				/>
				<ReportStat
					label={t("unanchoredSeals", "Unanchored seals")}
					value={report.unanchored_seals.toLocaleString()}
				/>
				<ReportStat
					label={t("redactedValues", "Expired values")}
					value={report.redacted_values.toLocaleString()}
				/>
				<ReportStat
					label={t("latestSeal", "Latest seal")}
					value={seqOrDash(report.latest_seal_seq)}
				/>
				<ReportStat
					label={t("latestEpoch", "Latest epoch")}
					value={seqOrDash(report.latest_epoch_seq)}
				/>
				<ReportStat
					label={t("prunedBefore", "Pruned through")}
					value={seqOrDash(report.pruned_before_seq)}
				/>
			</div>
			{report.unverifiable_epochs > 0 && (
				<p className="text-xs text-muted-foreground">
					{t(
						"reportUnverifiableEpochs",
						"{{epochs}} epochs were signed with a key this server cannot verify. Add its public key to AUDIT_VERIFYING_KEYS.",
						{ epochs: report.unverifiable_epochs },
					)}
				</p>
			)}
			{report.redacted_values > 0 && (
				<p className="text-xs text-muted-foreground">
					{t(
						"reportRedactedHint",
						"Expired IP addresses and details are reported as redacted: their commitments still verify.",
					)}
				</p>
			)}
		</div>
	);
}
