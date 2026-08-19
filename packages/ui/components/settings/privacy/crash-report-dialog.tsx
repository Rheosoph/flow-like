"use client";

import { useTranslation } from "@flow-like/locales";
import { Bug, Check, Copy, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { captureTelemetryError } from "../../../lib/telemetry/errors";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Label } from "../../ui/label";
import { Textarea } from "../../ui/textarea";

/**
 * Anonymous crash report. There is deliberately no name, no email and no
 * account reference: the report travels the same path as an automatic crash
 * report and is keyed to the random install id only.
 */
export interface IAnonymousCrashReport {
	/** Random id shown to the user so a report can be located in triage. */
	reportId: string;
	/** Internal id of the error the report is about, when the caller has one. */
	errorId?: string;
	description: string;
}

export interface CrashReportDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** Internal error id to attach; a fresh report id is generated when absent. */
	errorId?: string;
	/** False when crash reporting is off or unavailable — nothing can be sent. */
	reportingEnabled?: boolean;
	/** Overrides the default capture path; used by hosts with their own transport. */
	onSubmit?: (report: IAnonymousCrashReport) => void | Promise<void>;
}

/** Grouping title; the free text lives in the context so reports stay one issue. */
const REPORT_KIND = "UserCrashReport";
const REPORT_TITLE = "User-submitted crash report";
const REPORT_CULPRIT = "user-crash-report";
/** Stays under the per-string cap `sanitizeTelemetryContext` applies to context values. */
const DESCRIPTION_CHUNK_LENGTH = 240;
const MAX_DESCRIPTION_LENGTH = 960;

function createReportId(): string {
	try {
		if (
			typeof crypto !== "undefined" &&
			typeof crypto.randomUUID === "function"
		) {
			return crypto.randomUUID();
		}
	} catch {
		// Falls through to the non-crypto id below.
	}
	return `report-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

/** Splits the description so no chunk is truncated by context sanitization. */
function chunkDescription(description: string): string | string[] {
	if (description.length <= DESCRIPTION_CHUNK_LENGTH) return description;
	const chunks: string[] = [];
	for (
		let offset = 0;
		offset < description.length;
		offset += DESCRIPTION_CHUNK_LENGTH
	) {
		chunks.push(description.slice(offset, offset + DESCRIPTION_CHUNK_LENGTH));
	}
	return chunks;
}

/**
 * Sends an anonymous crash report through the regular crash-report queue. The
 * queue is gated on the crash-report consent, so a disabled setting drops the
 * report instead of delivering it.
 */
export function submitAnonymousCrashReport(report: IAnonymousCrashReport) {
	const description = report.description
		.trim()
		.slice(0, MAX_DESCRIPTION_LENGTH);
	const context: Record<string, unknown> = {
		report_kind: "user_crash_report",
		report_id: report.reportId,
	};
	if (report.errorId) context.error_id = report.errorId;
	if (description.length > 0)
		context.description = chunkDescription(description);

	captureTelemetryError(
		{ name: REPORT_KIND, message: REPORT_TITLE },
		{ level: "warning", culprit: REPORT_CULPRIT, context },
	);
}

function ReportReference({ reportId }: { readonly reportId: string }) {
	const [copied, setCopied] = useState(false);

	useEffect(() => {
		if (!copied) return;
		const timer = setTimeout(() => setCopied(false), 2000);
		return () => clearTimeout(timer);
	}, [copied]);

	const copy = useCallback(() => {
		navigator.clipboard
			?.writeText(reportId)
			.then(() => setCopied(true))
			.catch(() => undefined);
	}, [reportId]);

	return (
		<div className="flex items-center gap-2">
			<code className="flex-1 truncate rounded bg-muted px-1.5 py-1 font-mono text-[11px]">
				{reportId}
			</code>
			<Button type="button" variant="ghost" size="sm" onClick={copy}>
				{copied ? (
					<Check className="h-3.5 w-3.5" />
				) : (
					<Copy className="h-3.5 w-3.5" />
				)}
				{copied ? "Copied" : "Copy"}
			</Button>
		</div>
	);
}

export function CrashReportDialog({
	open,
	onOpenChange,
	errorId,
	reportingEnabled = true,
	onSubmit,
}: Readonly<CrashReportDialogProps>) {
	const { t } = useTranslation("settings");
	const [description, setDescription] = useState("");
	const [reportId, setReportId] = useState("");
	const [sent, setSent] = useState(false);

	useEffect(() => {
		if (!open) return;
		setDescription("");
		setSent(false);
		setReportId(errorId ?? createReportId());
	}, [open, errorId]);

	const send = useCallback(async () => {
		const report: IAnonymousCrashReport = {
			reportId,
			errorId,
			description,
		};
		try {
			await (onSubmit ? onSubmit(report) : submitAnonymousCrashReport(report));
		} catch {
			// Reporting is best-effort and must never throw into the application.
		}
		setSent(true);
	}, [description, errorId, onSubmit, reportId]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<Bug className="h-4 w-4" />
						{t("reportAProblem", "Report a problem")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"thisReportIsAnonymousItCarriesNoNameNoEmailAddressAndNoAccountInformationMdashOnlyWhatYouWriteBelowTheAppVersionAndTheRandomInstallId",
							"This report is anonymous. It carries no name, no email address and no account information — only what you write below, the app version and the random install id.",
						)}
					</DialogDescription>
				</DialogHeader>

				{sent ? (
					<div className="space-y-3">
						<p className="flex items-center gap-2 text-sm">
							<ShieldCheck className="h-4 w-4 text-primary" />
							{t(
								"thanksMdashTheAnonymousReportWasQueued",
								"Thanks — the anonymous report was queued.",
							)}
						</p>
						<div className="space-y-1">
							<Label className="text-xs text-muted-foreground">
								{t("referenceId", "Reference id")}
							</Label>
							<ReportReference reportId={reportId} />
						</div>
					</div>
				) : (
					<div className="space-y-4">
						<div className="space-y-1.5">
							<Label htmlFor="crash-report-description">
								{t("whatHappened", "What happened?")}
							</Label>
							<Textarea
								id="crash-report-description"
								value={description}
								maxLength={MAX_DESCRIPTION_LENGTH}
								rows={5}
								placeholder={t(
									"whatWereYouDoingWhenItBrokePleaseLeaveOutPersonalDetails",
									"What were you doing when it broke? Please leave out personal details.",
								)}
								onChange={(event) => setDescription(event.target.value)}
							/>
							<p className="text-right text-[11px] tabular-nums text-muted-foreground">{`${description.length}/${MAX_DESCRIPTION_LENGTH}`}</p>
						</div>

						<div className="space-y-1">
							<Label className="text-xs text-muted-foreground">
								{t("referenceId", "Reference id")}
							</Label>
							<ReportReference reportId={reportId} />
						</div>

						{!reportingEnabled && (
							<p className="text-sm text-muted-foreground">
								Crash &amp; error reports are turned off, so nothing can be
								sent. You can enable them under Settings &rsaquo; Privacy.
							</p>
						)}
					</div>
				)}

				<DialogFooter>
					{sent ? (
						<Button type="button" onClick={() => onOpenChange(false)}>
							{t("close", "Close")}
						</Button>
					) : (
						<>
							<Button
								type="button"
								variant="outline"
								onClick={() => onOpenChange(false)}
							>
								{t("cancel", "Cancel")}
							</Button>
							<Button
								type="button"
								disabled={!reportingEnabled || description.trim().length === 0}
								onClick={() => void send()}
							>
								{t("sendAnonymousReport", "Send anonymous report")}
							</Button>
						</>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
