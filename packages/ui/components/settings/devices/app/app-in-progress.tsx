"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Activity,
	Layers,
	type LucideIcon,
	Rocket,
	Upload,
} from "lucide-react";
import type { ReactNode } from "react";
import type { AppServiceRow } from "../../../../lib/device-management/model/app-plan";
import { rolloutEndsAt } from "../../../../lib/device-management/model/device-view";
import type { CopyRef } from "../../../../lib/device-management/model/types";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import {
	FreshnessStamp,
	MixedSourcesStamp,
} from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { useDevicesRoute } from "../routing/use-devices-route";
import { useActivityTray } from "../shell/activity-tray";
import { stampOf } from "../shell/attention-popover";
import {
	APP_LINKS,
	LinkButton,
	useAppPage,
	useDeviceNames,
} from "./app-shared";
import {
	type AppRun,
	type ServiceUpload,
	stagedEndsAt,
	stagedVersionOf,
	versionName,
} from "./app-view-local";
import { useStagedActions } from "./use-service-actions";

const ACTIVE_ROLLOUT = new Set(["validating", "activating", "rolling_back"]);

/** A service whose update is switching over right now. */
export const isUpdating = (row: AppServiceRow) =>
	!!row.view.rollout && ACTIVE_ROLLOUT.has(row.view.rollout.state);

function Row({
	icon: Icon,
	kind,
	title,
	sub,
	actions,
}: Readonly<{
	icon: LucideIcon;
	kind: string;
	title: ReactNode;
	sub?: ReactNode;
	actions?: ReactNode;
}>) {
	return (
		<li
			data-progress={kind}
			className="flex flex-wrap items-start gap-x-4 gap-y-2 border-t border-hairline px-4 py-2.5 first:border-t-0"
		>
			<div className="flex min-w-0 flex-[1_1_360px] items-start gap-4">
				<Icon aria-hidden className="mt-0.5 size-4 shrink-0 text-info" />
				<div className="flex min-w-0 flex-col gap-0.5">
					<p className="text-ui text-foreground">{title}</p>
					{sub ? <p className="text-xs text-muted-foreground">{sub}</p> : null}
				</div>
			</div>
			{actions ? (
				<div className="flex flex-wrap items-center gap-2 @max-[620px]/app:pl-8">
					{actions}
				</div>
			) : null}
		</li>
	);
}

function Follow() {
	const { t } = useTranslation("devices");
	return (
		<DvButton
			size="sm"
			icon={Activity}
			data-act="follow"
			onClick={() => useActivityTray.getState().setOpen(true)}
		>
			{t("app.progress.follow", "Follow")}
		</DvButton>
	);
}

const Name = ({ children }: Readonly<{ children?: ReactNode }>) => (
	<b className="font-mono font-semibold">{children}</b>
);
const Mono = ({ children }: Readonly<{ children?: ReactNode }>) => (
	<span className="font-mono">{children}</span>
);

function UpdatingRow({ row }: Readonly<{ row: AppServiceRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const deviceName = useDeviceNames();
	const rollout = row.view.rollout;
	if (!rollout) return null;
	const endsAt = rolloutEndsAt(rollout);
	const phase = {
		validating: t("app.progress.phaseChecking", "Checking the new version"),
		activating: t("app.progress.phaseSwitching", "Switching over"),
		rolling_back: t(
			"app.progress.phaseRestoring",
			"Restoring the previous version",
		),
	}[rollout.state as "validating" | "activating" | "rolling_back"];
	const from = rollout.base_revision;
	const to =
		rollout.active_revision ?? (from === undefined ? undefined : from + 1);
	return (
		<Row
			icon={Rocket}
			kind="update"
			title={
				endsAt && endsAt > time.nowS ? (
					<Trans
						t={t}
						i18nKey="app.progress.updatingLeft"
						defaults="Safe update · <1/> on <2/> · {{phase}} · {{time}} left"
						values={{ phase, time: time.countdown(endsAt) }}
						components={{
							1: <Name>{row.serviceId}</Name>,
							2: <Mono>{deviceName(row.deviceId)}</Mono>,
						}}
					/>
				) : (
					<Trans
						t={t}
						i18nKey="app.progress.updating"
						defaults="Safe update · <1/> on <2/> · {{phase}}"
						values={{ phase }}
						components={{
							1: <Name>{row.serviceId}</Name>,
							2: <Mono>{deviceName(row.deviceId)}</Mono>,
						}}
					/>
				)
			}
			sub={
				from !== undefined && to !== undefined
					? endsAt
						? t(
								"app.progress.updateSub",
								"Settings v{{from}} → v{{to}}. If it isn't healthy by {{time}}, the device restores v{{from}} on its own.",
								{ from, to, time: time.clock(endsAt) },
							)
						: t(
								"app.progress.updateSubNoTime",
								"Settings v{{from}} → v{{to}}.",
								{
									from,
									to,
								},
							)
					: undefined
			}
			actions={
				<>
					<Follow />
					<LinkButton
						route={APP_LINKS.service(row.deviceId, row.serviceId)}
						variant="ghost"
						size="sm"
						act="open-service"
					>
						{t("app.progress.openService", "Open service")}
					</LinkButton>
				</>
			}
		/>
	);
}

function StagedRow({ row }: Readonly<{ row: AppServiceRow }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const time = useAreaTime();
	const deviceName = useDeviceNames();
	const staged = stagedVersionOf(row, view.versions);
	const actions = useStagedActions(row, deviceName(row.deviceId), staged);
	const endsAt = stagedEndsAt(row);
	const from = row.version ? versionName(row.version) : null;
	return (
		<Row
			icon={Rocket}
			kind="staged"
			title={
				from && staged ? (
					<Trans
						t={t}
						i18nKey="app.progress.stagedVersions"
						defaults="Update ready · <1/> on <2/> · {{from}} → {{to}} · not active yet"
						values={{ from, to: versionName(staged) }}
						components={{
							1: <Name>{row.serviceId}</Name>,
							2: <Mono>{deviceName(row.deviceId)}</Mono>,
						}}
					/>
				) : (
					<Trans
						t={t}
						i18nKey="app.progress.staged"
						defaults="Update ready · <1/> on <2/> · not active yet"
						components={{
							1: <Name>{row.serviceId}</Name>,
							2: <Mono>{deviceName(row.deviceId)}</Mono>,
						}}
					/>
				)
			}
			sub={
				endsAt
					? t(
							"app.progress.stagedSub",
							"Discarded automatically on {{time}}.",
							{ time: time.at(endsAt) },
						)
					: undefined
			}
			actions={
				<>
					<GatedAction gate={actions.activateGate}>
						<DvButton
							size="sm"
							icon={Rocket}
							data-act="activate"
							onClick={actions.activate}
						>
							{t("app.progress.activate", "Activate…")}
						</DvButton>
					</GatedAction>
					<GatedAction gate={actions.discardGate}>
						<DvButton
							size="sm"
							variant="ghost"
							data-act="discard"
							onClick={actions.discard}
						>
							{t("app.progress.discard", "Discard…")}
						</DvButton>
					</GatedAction>
				</>
			}
		/>
	);
}

function UploadRow({ upload }: Readonly<{ upload: ServiceUpload }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const time = useAreaTime();
	const deviceName = useDeviceNames();
	const { deviceId } = upload.item.target;
	const device = upload.item.target.deviceName ?? deviceName(deviceId);
	const values = { device, app: view.app.name };
	const paused = upload.state === "paused";
	return (
		<Row
			icon={Upload}
			kind="upload"
			title={
				paused
					? t(
							"app.progress.uploadPaused",
							"Upload paused · {{device}} · {{app}}",
							values,
						)
					: t(
							"app.progress.uploading",
							"Uploading · {{device}} · {{app}}",
							values,
						)
			}
			sub={[
				upload.total > 0
					? t(
							"app.progress.files",
							"{{done, number}} of {{total, number}} files",
							{ done: upload.done, total: upload.total },
						)
					: "",
				paused && upload.expiresAt > time.nowS
					? t("app.progress.resumableUntil", "resumable until {{time}}", {
							time: time.at(upload.expiresAt),
						})
					: "",
			]
				.filter(Boolean)
				.join(" · ")}
			actions={
				<>
					<LinkButton
						route={APP_LINKS.deploy({
							deviceIds: [deviceId],
							mode: "update",
							step: "copy_upload",
						})}
						size="sm"
						icon={Upload}
						act="resume-upload"
					>
						{upload.state === "paused"
							? t("app.progress.resume", "Resume in deploy")
							: t("app.progress.openDeploy", "Open in deploy")}
					</LinkButton>
					<Follow />
				</>
			}
		/>
	);
}

interface RunWords {
	what: string;
	count: number;
}

function runWords(title: CopyRef<string>, total: number): RunWords {
	const params = title.params ?? {};
	return {
		what: String(params.what ?? ""),
		count: Number(params.count) || total,
	};
}

const updateTitle = (t: DevicesT, words: RunWords) =>
	words.what
		? t(
				"devices:app.progress.runUpdate",
				"Update {{what}} on {{count, number}} devices",
				{ what: words.what, count: words.count },
			)
		: t(
				"devices:app.progress.runUpdateAny",
				"Update on {{count, number}} devices",
				{ count: words.count },
			);

const deployTitle = (t: DevicesT, words: RunWords) =>
	words.what
		? t(
				"devices:app.progress.runDeploy",
				"Deploy {{what}} to {{count, number}} devices",
				{ what: words.what, count: words.count },
			)
		: t(
				"devices:app.progress.runDeployAny",
				"Deploy to {{count, number}} devices",
				{ count: words.count },
			);

/** The run's title as the deploy flow keeps it: `deploy` or `update` with `what` and `count`. */
export function runTitle(
	t: DevicesT,
	title: CopyRef<string>,
	total: number,
): string {
	const words = runWords(title, total);
	return title.code === "update"
		? updateTitle(t, words)
		: deployTitle(t, words);
}

function RunRow({ entry }: Readonly<{ entry: AppRun }>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const href = entry.items.find((item) => item.href)?.href;
	return (
		<Row
			icon={Layers}
			kind="run"
			title={t(
				"app.progress.runDone",
				"{{title}} · {{done, number}} of {{total, number}} done",
				{
					title: runTitle(t, entry.run.title, entry.total),
					done: entry.done,
					total: entry.total,
				},
			)}
			actions={
				<>
					<Follow />
					{href ? (
						<DvButton
							size="sm"
							variant="ghost"
							data-act="open-run"
							onClick={() => navigate(href)}
						>
							{t("app.progress.open", "Open")}
						</DvButton>
					) : null}
				</>
			}
		/>
	);
}

/** APP §2.8: every running or resumable operation of this app, newest first; hidden when there is none. */
export function AppInProgress({
	updateRun,
	skipRunId,
}: Readonly<{
	/** The running Update everywhere, as its full rollout block. */
	updateRun?: ReactNode;
	/** The tray run of that block, so it isn't listed twice. */
	skipRunId?: string;
}>) {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const updating = view.services.filter(isUpdating);
	const staged = view.services.filter((row) => row.staged);
	const runs = data.runs.filter((entry) => entry.run.id !== skipRunId);
	const live = updating.length + staged.length + (updateRun ? 1 : 0);
	const local = data.uploads.length + runs.length;
	if (!live && !local) return null;
	const first = updating[0] ?? staged[0];
	// Without a service row to vouch for it, the block holds what this computer tracks (the Update everywhere run).
	const tracked = (
		<FreshnessStamp
			source="local"
			age="current"
			text={t("app.progress.tracked", "tracked on this computer")}
		/>
	);
	return (
		<Block
			id="ad-prog"
			icon={Activity}
			title={t("app.progress.title", "In progress")}
			count={live + local}
			flush
			stamp={
				first && local ? (
					<MixedSourcesStamp />
				) : first ? (
					<FreshnessStamp {...stampOf(first.view.freshness)} />
				) : (
					tracked
				)
			}
		>
			<ul className="m-0 list-none p-0">
				{updating.map((row) => (
					<UpdatingRow key={`${row.deviceId}/${row.serviceId}`} row={row} />
				))}
				{data.uploads.map((upload) => (
					<UploadRow key={upload.item.id} upload={upload} />
				))}
				{runs.map((entry) => (
					<RunRow key={entry.run.id} entry={entry} />
				))}
				{staged.map((row) => (
					<StagedRow key={`${row.deviceId}/${row.serviceId}`} row={row} />
				))}
			</ul>
			{updateRun ? (
				<div className="border-t border-hairline p-3">{updateRun}</div>
			) : null}
		</Block>
	);
}
