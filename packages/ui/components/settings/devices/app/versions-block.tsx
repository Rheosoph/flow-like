"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, GitCommitHorizontal, Layers } from "lucide-react";
import { Fragment, useState } from "react";
import type {
	AppDeviceGroup,
	AppServiceRow,
	AppVersionView,
	VersionDiffRow,
	VersionRunning,
} from "../../../../lib/device-management/model/app-plan";
import { appCopy } from "../copy/app-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { PairedPins } from "../primitives/paired-pins";
import { StatusChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import {
	APP_LINKS,
	LINK,
	Person,
	useAppPage,
	useCapped,
	useDayTime,
	useDeviceNames,
} from "./app-shared";
import {
	UNKNOWN_FOLD,
	VERSION_CAP,
	VERSION_RUN_CAP,
	capList,
	pinText,
	versionName,
} from "./app-view-local";
import { desiredRun, observedRun } from "./where-by-device";

function diffPart(
	t: DevicesT,
	row: VersionDiffRow,
	name: string,
): string | null {
	if (row.kind === "added")
		return t("devices:app.versions.diffNew", "new: {{event}}", {
			event: name,
		});
	if (row.kind === "removed")
		return t("devices:app.versions.diffRemoved", "removed: {{event}}", {
			event: name,
		});
	if (!row.from || !row.to) return null;
	const from = pinText(row.from.eventVersion);
	const to = pinText(row.to.eventVersion);
	return from === to
		? t("devices:app.versions.diffFlow", "{{event}} flow {{from}} → {{to}}", {
				event: name,
				from: pinText(row.from.boardVersion),
				to: pinText(row.to.boardVersion),
			})
		: t("devices:app.versions.diffEvent", "{{event}} {{from}} → {{to}}", {
				event: name,
				from,
				to,
			});
}

function VersionDiff({
	version,
	older,
}: Readonly<{ version: AppVersionView; older: AppVersionView | undefined }>) {
	const { t } = useTranslation("devices");
	const { data } = useAppPage();
	if (!version.diff || !older) return null;
	const parts = version.diff
		.map((row) =>
			diffPart(t, row, data.eventNames.get(row.eventId) ?? row.eventId),
		)
		.filter((part): part is string => !!part);
	return (
		<p data-ver-diff="" className="text-ui text-ink-2">
			{parts.length
				? t("app.versions.since", "Since {{version}}: {{changes}}", {
						version: versionName(older),
						changes: parts.join(" · "),
					})
				: t(
						"app.versions.sameEvents",
						"Same events; packages or models changed.",
					)}
		</p>
	);
}

const MUTED = "text-xs text-muted-foreground";

type UnknownKind = "locked" | "nokeys" | "other";

function unknownKind(group: AppDeviceGroup | undefined): UnknownKind {
	const kind = group?.unknown?.kind;
	if (kind === "nokeys") return "nokeys";
	return kind && kind !== "locked" ? "other" : "locked";
}

/** Devices whose status can't be read: named one by one, or one sentence past `UNKNOWN_FOLD` (R11). */
function UnknownDevices({
	deviceIds,
}: Readonly<{ deviceIds: readonly string[] }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const deviceName = useDeviceNames();
	const kinds = deviceIds.map((deviceId) =>
		unknownKind(view.groups.find((group) => group.deviceId === deviceId)),
	);
	if (deviceIds.length <= UNKNOWN_FOLD) {
		const why: Record<UnknownKind, string> = {
			locked: t("app.versions.unknownOn", " · unknown until unlocked"),
			nokeys: t("app.versions.unknownNoKeys", " · no keys here"),
			other: t("app.versions.unknownStatus", " · status unknown"),
		};
		return deviceIds.map((deviceId, index) => (
			<span key={deviceId} className={MUTED}>
				<span className="font-mono">{deviceName(deviceId)}</span>
				{why[kinds[index] ?? "locked"]}
			</span>
		));
	}
	const count = (kind: UnknownKind) =>
		kinds.filter((entry) => entry === kind).length;
	const total = deviceIds.length;
	const parts = [
		count("locked")
			? t("app.versions.foldLocked", "{{count, number}} locked", {
					count: count("locked"),
				})
			: "",
		count("nokeys")
			? t("app.versions.foldNoKeys", "{{count, number}} without keys here", {
					count: count("nokeys"),
				})
			: "",
		count("other")
			? t("app.versions.foldOther", "{{count, number}} not readable", {
					count: count("other"),
				})
			: "",
	].filter(Boolean);
	return (
		<span data-ver-fold="devices" className={MUTED}>
			{count("locked") === total
				? t(
						"app.versions.foldAllLocked",
						"{{count, number}} devices unknown until unlocked",
						{ count: total },
					)
				: t(
						"app.versions.foldMixed",
						"{{count, number}} devices unknown: {{parts}}",
						{ count: total, parts: parts.join(", ") },
					)}
		</span>
	);
}

/** Services whose version can't be told (the status snapshot has no event list): never "not running". */
function UnknownServices({
	rows,
}: Readonly<{ rows: readonly AppServiceRow[] }>) {
	const { t } = useTranslation("devices");
	const deviceName = useDeviceNames();
	if (rows.length > UNKNOWN_FOLD)
		return (
			<span data-ver-fold="services" className={MUTED}>
				{t(
					"app.versions.foldServices",
					"{{count, number}} services · version unknown",
					{ count: rows.length },
				)}
			</span>
		);
	return rows.map((row) => (
		<span
			key={`${row.deviceId}/${row.serviceId}`}
			data-ver-unknown={row.serviceId}
			className={MUTED}
		>
			<span className="font-mono">
				{deviceName(row.deviceId)} › {row.serviceId}
			</span>
			{t("app.versions.versionUnknown", " · version unknown")}
		</span>
	));
}

/** One service that runs the version: its state glyph, device › service. */
function RunLink({ entry }: Readonly<{ entry: VersionRunning }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const link = useRouteLink();
	const deviceName = useDeviceNames();
	const row = view.services.find(
		(service) =>
			service.deviceId === entry.deviceId &&
			service.serviceId === entry.serviceId,
	);
	return (
		<span
			data-ver-run={entry.serviceId}
			className="inline-flex min-w-0 items-center gap-1.5 font-mono text-xs"
		>
			{row ? (
				<PairedPins
					desired={desiredRun(row.view.desired)}
					observed={observedRun(row.view.observed)}
					conv={row.view.conv}
				/>
			) : null}
			<a {...link(APP_LINKS.device(entry.deviceId))} className={LINK}>
				{deviceName(entry.deviceId)}
			</a>
			<span aria-hidden className="text-muted-foreground">
				›
			</span>
			<a
				{...link(APP_LINKS.service(entry.deviceId, entry.serviceId))}
				className={LINK}
			>
				{entry.serviceId}
			</a>
			{entry.lastKnown ? (
				<span className="font-sans text-muted-foreground">
					{t("app.versions.lastKnown", "· last known")}
				</span>
			) : null}
		</span>
	);
}

/** No service is known to run the version: said as far as the page can vouch for it. */
function notRunning(
	t: DevicesT,
	newest: boolean,
	unknownDevices: number,
	unknownServices: number,
): string {
	if (unknownDevices)
		return t(
			"devices:app.versions.notRunningSeen",
			"Not running on any device you can see",
		);
	if (unknownServices)
		return t(
			"devices:app.versions.notRunningKnown",
			"Not running on any service whose version is known",
		);
	return newest
		? t("devices:app.versions.notRunningYet", "Not running anywhere yet")
		: t("devices:app.versions.notRunning", "Not running anywhere");
}

function VersionRuns({ version }: Readonly<{ version: AppVersionView }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const runs = useCapped(version.runningOn, VERSION_RUN_CAP);
	const unknownDevices = version.unknownOn.filter((key) => !key.includes("/"));
	const unknownServices = view.services.filter((row) => !row.version);
	return (
		<p
			data-ver-runs=""
			className="flex flex-wrap items-center gap-x-3 gap-y-1 text-ui text-ink-2"
		>
			{version.runningOn.length === 0 ? (
				<span className="text-muted-foreground">
					{notRunning(
						t,
						version.index === 0,
						unknownDevices.length,
						unknownServices.length,
					)}
				</span>
			) : (
				<>
					{runs.shown.map((entry) => (
						<RunLink
							key={`${entry.deviceId}/${entry.serviceId}`}
							entry={entry}
						/>
					))}
					{runs.rest.length ? (
						<DvButton
							variant="link"
							size="xs"
							data-act="ad-ver-more"
							onClick={runs.showAll}
						>
							{t("app.versions.moreRuns", {
								count: runs.rest.length,
								defaultValue_one: "and {{count, number}} more service",
								defaultValue_other: "and {{count, number}} more services",
							})}
						</DvButton>
					) : null}
				</>
			)}
			<UnknownDevices deviceIds={unknownDevices} />
			<UnknownServices rows={unknownServices} />
		</p>
	);
}

function VersionMeta({ version }: Readonly<{ version: AppVersionView }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const dayTime = useDayTime();
	const events = t("app.versions.events", {
		count: version.pins.length,
		defaultValue_one: "{{count, number}} event",
		defaultValue_other: "{{count, number}} events",
	});
	const when = version.builtAt ? dayTime(version.builtAt) : null;
	const local = view.app.localOnly;
	return (
		<p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
			{when ? (
				<span>
					{version.index > 0 && !version.label
						? t("app.versions.sent", "Sent from this computer {{when}}", {
								when,
							})
						: local
							? t("app.versions.changed", "Changed on this computer {{when}}", {
									when,
								})
							: t("app.versions.built", "Built {{when}}", { when })}
				</span>
			) : null}
			{version.by && !local ? (
				<>
					<span>{t("app.versions.by", "by")}</span>
					<Person userId={version.by} />
				</>
			) : null}
			{when ? <span aria-hidden>·</span> : null}
			<span>{events}</span>
		</p>
	);
}

function VersionRow({
	version,
	older,
}: Readonly<{ version: AppVersionView; older: AppVersionView | undefined }>) {
	const { t } = useTranslation("devices");
	const { openUpdateAll, updateAllGate } = useAppPage();
	const copy = appCopy(t);
	const newest = version.index === 0;
	return (
		<li
			data-version={version.short}
			data-fresh={newest || undefined}
			className="flex flex-wrap items-start justify-between gap-x-6 gap-y-2 border-t border-hairline px-4 py-3 first:border-t-0"
		>
			<div className="flex min-w-0 flex-[1_1_420px] flex-col gap-1.5">
				<p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-ui">
					{version.label ? (
						<span className="font-mono text-ui">{version.label}</span>
					) : null}
					<IdRef
						id={version.hash}
						copyLabel={t("app.versions.copyHash", "Copy app version hash")}
					/>
					{newest ? (
						<StatusChip tone="good" icon={Check}>
							{copy.drift(0)}
						</StatusChip>
					) : null}
				</p>
				<VersionMeta version={version} />
				<VersionDiff version={version} older={older} />
				<VersionRuns version={version} />
			</div>
			<div className="flex max-w-[34ch] min-w-0 flex-col items-end text-right @max-[720px]/app:items-start @max-[720px]/app:text-left">
				{newest ? (
					<GatedAction
						gate={updateAllGate}
						className="items-end @max-[720px]/app:items-start"
					>
						<DvButton
							size="sm"
							icon={Layers}
							data-act="ad-roll-out"
							onClick={openUpdateAll}
						>
							{t("app.versions.rollOut", "Roll out…")}
						</DvButton>
					</GatedAction>
				) : (
					<p className="text-xs text-muted-foreground">{copy.olderVersion()}</p>
				)}
			</div>
		</li>
	);
}

/** APP §2.11: the app's versions, newest first, with where each one runs. */
export function VersionsBlock() {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const time = useAreaTime();
	const [all, setAll] = useState(false);
	const { versions } = view;
	const checkedAt = data.readAt;
	if (!view.app.canReadFlows || !versions.length) return null;
	const { shown, rest } = all
		? { shown: versions, rest: [] }
		: capList(versions, VERSION_CAP);
	const local = view.app.localOnly;
	return (
		<Block
			id="ad-versions"
			icon={GitCommitHorizontal}
			title={t("app.versions.title", "Versions")}
			count={versions.length}
			flush
			stamp={
				local ? (
					<FreshnessStamp
						source="local"
						age="current"
						text={t("app.versions.stampLocal", "on this computer")}
					/>
				) : (
					<FreshnessStamp
						source="hub"
						age="current"
						text={
							checkedAt
								? t(
										"app.versions.stampHubChecked",
										"published versions · checked {{ago}}",
										{ ago: time.ago(checkedAt) },
									)
								: t("app.versions.stampHub", "published versions")
						}
						{...(checkedAt ? { observedAt: checkedAt } : {})}
					/>
				)
			}
			foot={
				<span>
					{local
						? t(
								"app.versions.footLocal",
								"An app version pins one version of each event and its flow. A device gets a new version only when you send it a new copy.",
							)
						: t(
								"app.versions.footOnline",
								"An app version pins one version of each event and its flow. A device gets a new version only when you re-pin its service.",
							)}
				</span>
			}
		>
			<ul className="m-0 flex list-none flex-col p-0">
				{shown.map((version) => (
					<Fragment key={version.hash}>
						<VersionRow version={version} older={versions[version.index + 1]} />
					</Fragment>
				))}
			</ul>
			{rest.length || all ? (
				<div
					className={cx(
						"border-t border-hairline px-4 py-2",
						versions.length <= VERSION_CAP && "hidden",
					)}
				>
					<DvButton
						variant="link"
						size="xs"
						aria-expanded={all}
						onClick={() => setAll((value) => !value)}
					>
						{all
							? t("app.versions.showFewer", "Show fewer")
							: t("app.versions.showOlder", "Show {{count, number}} older", {
									count: rest.length,
								})}
					</DvButton>
				</div>
			) : null}
		</Block>
	);
}
