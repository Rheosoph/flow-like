"use client";

import { useTranslation } from "@flow-like/locales";
import { CloudDownload, X } from "lucide-react";
import type { ReactNode } from "react";
import type { ModelJob } from "../../../../lib/device-management/models";
import { errorCopy } from "../copy/error-copy";
import { gateView } from "../device/use-device-page";
import { bytesText } from "../observe/observe-data";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { ProgressBar } from "../primitives/meter";
import { StatusChip } from "../primitives/status-chip";
import { type ChipTone, TONE_TEXT, cx } from "../primitives/tone";
import { stampOf } from "../shell/attention-popover";
import { useInlineResults } from "../workspace";
import {
	etaText,
	jobFailureText,
	jobStateLabel,
	rateText,
} from "./models-copy";
import { jobActive, jobProgress, orderJobs } from "./models-view";
import type { ModelJobs, ModelsRead } from "./use-models";
import { modelsResultKey, useModelsAction } from "./use-models-action";

const JOB_TONE: Record<ModelJob["state"], ChipTone> = {
	queued: "outline",
	fetching: "info",
	verifying: "info",
	present: "good",
	awaiting_push: "warning",
	failed: "critical",
};

/** "1.0 GiB of 2.3 GiB · 50.0 MiB/s · about 26 s left". */
function progressLine(t: DevicesT, job: ModelJob): string | undefined {
	const progress = jobProgress(job);
	if (progress.bytes === undefined) return undefined;
	const parts = [
		t("devices:models.downloads.bytes", "{{done}} of {{total}}", {
			done: bytesText(progress.bytes),
			total: bytesText(job.size),
		}),
	];
	if (job.state === "fetching" && job.bytes_per_second)
		parts.push(rateText(t, job.bytes_per_second));
	if (progress.etaS !== undefined)
		parts.push(
			t("devices:models.downloads.left", "{{eta}} left", {
				eta: etaText(t, progress.etaS),
			}),
		);
	return parts.join(" · ");
}

function CancelButton({
	deviceId,
	job,
}: Readonly<{ deviceId: string; job: ModelJob }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const command = useModelsAction(deviceId).cancelJob(job);
	const view = gateView(t, time, command.gate);
	return (
		<GatedAction gate={view?.gate}>
			<DvButton
				size="xs"
				variant="ghost"
				icon={X}
				busy={command.pending}
				onClick={() => void command.run()}
			>
				{t("devices:models.actions.cancelJobButton", "Cancel…")}
			</DvButton>
		</GatedAction>
	);
}

interface JobRowProps {
	deviceId: string;
	job: ModelJob;
	sendFromThisComputer?: (job: ModelJob) => ReactNode;
}

function JobRow({
	deviceId,
	job,
	sendFromThisComputer,
}: Readonly<JobRowProps>) {
	const { t } = useTranslation("devices");
	const results = useInlineResults(modelsResultKey.job(deviceId, job.job_id));
	const progress = jobProgress(job);
	const line = progressLine(t, job);
	return (
		<li
			data-job={job.job_id}
			className="flex flex-col gap-1.5 border-t border-hairline px-4 py-3 first:border-t-0"
		>
			<span className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
				<span className="min-w-0 font-mono text-ui wrap-anywhere">
					{job.file_name}
				</span>
				<StatusChip tone={JOB_TONE[job.state]}>
					{jobStateLabel(t, job.state)}
				</StatusChip>
				{job.source_host ? (
					<span className="text-xs text-muted-foreground">
						{t("devices:models.downloads.from", "from {{host}}", {
							host: job.source_host,
						})}
					</span>
				) : null}
				<span className="flex-1" />
				{jobActive(job) ? <CancelButton deviceId={deviceId} job={job} /> : null}
			</span>
			{progress.percent !== undefined && job.state !== "present" ? (
				<ProgressBar
					value={progress.percent}
					tone={job.state === "awaiting_push" ? "warning" : "info"}
					label={t(
						"devices:models.downloads.progress",
						"Download of {{file}}",
						{
							file: job.file_name,
						},
					)}
					className="max-w-[420px]"
				/>
			) : null}
			{line ? (
				<span className="text-xs tabular-nums text-muted-foreground">
					{line}
				</span>
			) : null}
			{job.state === "failed" ? (
				<span className={cx("text-xs", TONE_TEXT.critical)}>
					{jobFailureText(t, job)}
				</span>
			) : null}
			{sendFromThisComputer?.(job)}
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</li>
	);
}

export interface DownloadsBlockProps {
	deviceId: string;
	device: string;
	jobs: ModelsRead<ModelJobs>;
	/**
	 * Lane C1: "Send from this computer", called for every download. It shows
	 * under one the device can't finish itself (failed, or waiting for a push)
	 * and stays while this computer sends, also while the device verifies.
	 */
	sendFromThisComputer?: (job: ModelJob) => ReactNode;
}

/** Model files and runtime packs the device downloads: progress, source, rate and why one failed. */
export function DownloadsBlock({
	deviceId,
	device,
	jobs,
	sendFromThisComputer,
}: Readonly<DownloadsBlockProps>) {
	const { t } = useTranslation("devices");
	const all = jobs.data?.jobs ?? [];
	const shown = orderJobs(all.filter((job) => job.state !== "present"));
	const finished = all.length - shown.length;
	if (!shown.length && !jobs.failure) return null;
	return (
		<Block
			id="models-downloads"
			icon={CloudDownload}
			title={t("devices:models.downloads.title", "Downloads")}
			count={shown.length}
			stamp={<FreshnessStamp {...stampOf(jobs.freshness)} />}
			flush
			foot={
				finished
					? t("devices:models.downloads.finished", {
							count: finished,
							defaultValue_one: "{{count, number}} download finished.",
							defaultValue_other: "{{count, number}} downloads finished.",
						})
					: null
			}
		>
			{shown.length ? (
				<ul className="flex flex-col">
					{shown.map((job) => (
						<JobRow
							key={job.job_id}
							deviceId={deviceId}
							job={job}
							sendFromThisComputer={sendFromThisComputer}
						/>
					))}
				</ul>
			) : (
				<p className="px-4 py-3 text-ui text-muted-foreground">
					{jobs.failure ? errorCopy(t, jobs.failure.code, { device }) : null}
				</p>
			)}
		</Block>
	);
}
