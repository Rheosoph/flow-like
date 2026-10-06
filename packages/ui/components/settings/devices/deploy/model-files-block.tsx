"use client";

import { useTranslation } from "@flow-like/locales";
import { CloudDownload, Square } from "lucide-react";
import type { DeployRunState } from "../../../../lib/device-management/model/deploy-run";
import type { ModelJob } from "../../../../lib/device-management/models";
import { etaText, jobFailureText, rateText } from "../models/models-copy";
import { bytesText } from "../observe/observe-data";
import type { DevicesT } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { ProgressBar, type ProgressTone } from "../primitives/meter";
import { StatusChip } from "../primitives/status-chip";
import { type ChipTone, TONE_TEXT, cx } from "../primitives/tone";
import type { ModelFileRow, ModelFileState } from "./ensure-models";
import { type DeployRunDetails, useCancelModelPush } from "./use-deploy-run";

const CHIP: Record<ModelFileState, ChipTone> = {
	waiting: "outline",
	checking: "outline",
	downloading: "info",
	verifying: "info",
	sending: "warning",
	present: "good",
	failed: "critical",
};

const BAR: Partial<Record<ModelFileState, ProgressTone>> = {
	downloading: "info",
	verifying: "info",
	sending: "warning",
	failed: "critical",
};

function sendingLabel(t: DevicesT, file: ModelFileRow): string {
	if (file.from === "local_download")
		return t(
			"devices:deployShip.models.state.downloadingHere",
			"Downloading to this computer",
		);
	return file.from === "download"
		? t(
				"devices:deployShip.models.state.relaying",
				"Downloading through this computer",
			)
		: t(
				"devices:deployShip.models.state.sending",
				"Sending from this computer",
			);
}

function stateLabel(t: DevicesT, file: ModelFileRow): string {
	const labels = {
		waiting: t("devices:deployShip.models.state.waiting", "Waiting"),
		checking: t("devices:deployShip.models.state.checking", "Checking"),
		downloading: t(
			"devices:deployShip.models.state.downloading",
			"Device is downloading",
		),
		verifying: t("devices:deployShip.models.state.verifying", "Verifying"),
		sending: sendingLabel(t, file),
		present: t("devices:deployShip.models.state.present", "On the device"),
		failed: t("devices:deployShip.models.state.failed", "Failed"),
	} satisfies Record<ModelFileState, string>;
	return labels[file.state];
}

/** "2.1 GiB of 4.9 GiB · from cdn.flow-like.com · 50 MiB/s · about 1 min left". */
function progressLine(t: DevicesT, file: ModelFileRow): string | undefined {
	if (file.state === "present" || file.state === "failed") return undefined;
	if (file.state === "waiting" || file.state === "checking") return undefined;
	const parts = [
		t("devices:deployShip.models.bytes", "{{done}} of {{total}}", {
			done: bytesText(file.bytes),
			total: bytesText(file.size),
		}),
	];
	if (file.state === "downloading" && file.sourceHost)
		parts.push(
			t("devices:deployShip.models.from", "from {{host}}", {
				host: file.sourceHost,
			}),
		);
	if (file.bytesPerSecond) {
		parts.push(rateText(t, file.bytesPerSecond));
		parts.push(
			t("devices:deployShip.models.left", "{{eta}} left", {
				eta: etaText(t, (file.size - file.bytes) / file.bytesPerSecond),
			}),
		);
	}
	return parts.join(" · ");
}

/** Why the file is missing: the device's own reason, or why sending it from here failed. */
function failureLine(t: DevicesT, file: ModelFileRow): string | undefined {
	if (file.state !== "failed") return undefined;
	if (file.error)
		return t(
			"devices:deployShip.models.sendFailed",
			"Sending it from this computer failed: {{error}}",
			{ error: file.error },
		);
	if (!file.reason) return undefined;
	return jobFailureText(t, {
		state: "failed",
		reason: file.reason,
	} as Extract<ModelJob, { state: "failed" }>);
}

function FileRow({
	file,
	onStop,
}: Readonly<{ file: ModelFileRow; onStop?: () => void }>) {
	const { t } = useTranslation("devices");
	const line = progressLine(t, file);
	const failure = failureLine(t, file);
	const tone = BAR[file.state];
	return (
		<li
			data-model-file={file.key}
			className="flex flex-col gap-1.5 border-t border-hairline px-4 py-3 first:border-t-0"
		>
			<span className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
				<span className="min-w-0 font-mono text-ui wrap-anywhere">
					{file.fileName}
				</span>
				<StatusChip tone={CHIP[file.state]}>{stateLabel(t, file)}</StatusChip>
				<span className="flex-1" />
				{onStop ? (
					<DvButton size="xs" variant="ghost" icon={Square} onClick={onStop}>
						{t("devices:deployShip.models.stop", "Stop sending")}
					</DvButton>
				) : null}
			</span>
			{tone ? (
				<ProgressBar
					value={(file.bytes / file.size) * 100}
					tone={tone}
					label={t(
						"devices:deployShip.models.progress",
						"Transfer of {{file}}",
						{
							file: file.fileName,
						},
					)}
					className="max-w-105"
				/>
			) : null}
			{line ? (
				<span className="text-xs tabular-nums text-muted-foreground">
					{line}
				</span>
			) : null}
			{failure ? (
				<span className={cx("text-xs", TONE_TEXT.critical)}>{failure}</span>
			) : null}
		</li>
	);
}

/**
 * The model files each device gets for this version (plan §3.2): the device
 * downloads them itself, and a file it can't get is sent from this computer,
 * visibly and with a way to stop it.
 */
export function ModelFilesBlock({
	deploymentId,
	state,
	details,
}: Readonly<{
	deploymentId: string;
	state: DeployRunState;
	details: DeployRunDetails;
}>) {
	const { t } = useTranslation("devices");
	const stop = useCancelModelPush(deploymentId);
	const targets = state.rows.flatMap((row) => {
		const detail = details[row.target];
		const files = detail?.models ?? [];
		return files.some((file) => file.state !== "present")
			? [{ row, files, device: detail?.deviceName ?? row.deviceId }]
			: [];
	});
	if (!targets.length) return null;
	return (
		<Block
			id="dp-models"
			icon={CloudDownload}
			title={t("devices:deployShip.models.title", "Model files")}
			flush
		>
			{targets.map(({ row, files, device }) => (
				<section key={row.target} data-model-target={row.target}>
					{targets.length > 1 ? (
						<h3 className="border-t border-hairline px-4 pt-3 text-ui font-semibold first:border-t-0">
							{device}
						</h3>
					) : null}
					<ul className="flex flex-col">
						{files.map((file) => (
							<FileRow
								key={file.key}
								file={file}
								onStop={
									row.state === "active" && file.state === "sending"
										? () => stop(row.target)
										: undefined
								}
							/>
						))}
					</ul>
				</section>
			))}
		</Block>
	);
}
