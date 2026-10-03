"use client";

import { useTranslation } from "@flow-like/locales";
import { GitCompare, KeyRound } from "lucide-react";
import type { ReactNode } from "react";
import type { DeploymentRolloutStatus } from "../../../../lib/device-management/deployment";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	DeviceViewModel,
	PlacementStatusPlus,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { ActivityItem } from "../../../../lib/device-management/workspace/types";
import { appCopy } from "../copy/app-copy";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { IdRef } from "../primitives/id-ref";
import { RequestedActual } from "../primitives/requested-actual";
import { LOCKED_DATA_CLASS } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import { RunsYouStarted } from "../run/runs-you-started";
import { stampOf } from "../shell/attention-popover";
import { useActivity, useOverlayStore } from "../workspace";
import { ActionsBlock } from "./actions-block";
import { BotsBlock, botRows } from "./bots-block";
import {
	CurrentUpdate,
	type ServiceRollouts,
	UpdateResults,
} from "./current-update";
import { DiagnosisBlock, type ServiceDiagnosis } from "./diagnosis-banner";
import { type InstanceDiagnostics, InstancesTable } from "./instances-table";
import { SchedulesBlock, scheduleRows } from "./schedules-block";
import { appEventRows } from "./service-events";
import {
	AppVersionValue,
	type ServiceApp,
	SettingsVersion,
	desiredRun,
	observedRun,
	useModeText,
} from "./service-header";
import { UpdateHistory } from "./update-history";

const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";

/** Facts that stay side by side in this tab's narrow column; the shared list stacks below 520 px of its container. */
function Facts({
	className,
	children,
}: Readonly<{ className?: string; children: ReactNode }>) {
	return (
		<div className="@container/facts min-w-0">
			<dl
				className={cx(
					"grid grid-cols-[minmax(128px,max-content)_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-ui @max-[340px]/facts:grid-cols-1 @max-[340px]/facts:gap-y-0.5",
					className,
				)}
			>
				{children}
			</dl>
		</div>
	);
}

function Fact({
	label,
	note,
	children,
}: Readonly<{ label: ReactNode; note?: ReactNode; children: ReactNode }>) {
	return (
		<div className="contents">
			<dt className="text-muted-foreground @max-[340px]/facts:mt-1.5">
				{label}
			</dt>
			<dd className="m-0 min-w-0 wrap-anywhere">
				{children}
				{note ? (
					<span className="mt-0.5 block text-xs text-muted-foreground">
						{note}
					</span>
				) : null}
			</dd>
		</div>
	);
}

function RequestedVsActual({
	device,
	service,
	app,
	placement,
}: Readonly<{
	device: DeviceViewModel;
	service: ServiceView;
	app: ServiceApp;
	placement: PlacementStatusPlus | undefined;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const name = deviceName(device.row);
	const modeText = useModeText(app, name);
	const { applied, latest } = service.settings;
	const { ready, requested, running, max } = service.instances;
	const runs =
		service.observed === "running" || service.observed === "starting";
	return (
		<Block
			icon={GitCompare}
			title={t("service.requested.title", "Requested vs actual")}
			stamp={<FreshnessStamp {...stampOf(service.freshness)} />}
			foot={t(
				"service.requested.foot",
				"Requested is what you asked for; actual is what the device reports. The agent restarts crashed instances on its own, up to {{max}} times.",
				{ max: service.diagnostics?.restarts?.max_restarts ?? 5 },
			)}
		>
			<Facts>
				<Fact label={t("service.requested.state", "Requested → actual")}>
					<RequestedActual
						desired={desiredRun(service)}
						observed={observedRun(service)}
						conv={service.conv}
					/>
				</Fact>
				<Fact label={t("service.requested.settings", "Settings version")}>
					{applied !== null && applied !== latest ? (
						<SettingsVersion
							service={service}
							requestedLabel={t(
								"service.requested.latest",
								"Latest v{{version}}",
								{ version: latest },
							)}
						/>
					) : runs ? (
						t(
							"service.requested.settingsRunning",
							"v{{version}} · latest and running",
							{ version: latest },
						)
					) : (
						t(
							"service.requested.settingsApplied",
							"v{{version}} · latest and applied",
							{ version: latest },
						)
					)}
				</Fact>
				<Fact label={t("service.requested.instances", "Instances")}>
					<span className="tabular-nums">
						{t(
							"service.requested.instancesValue",
							"{{ready}} of {{requested}} ready · {{running}} running · max {{max}}",
							{ ready, requested, running, max },
						)}
					</span>
				</Fact>
				<Fact label={t("service.requested.version", "App version")}>
					<span className="inline-flex flex-wrap items-center gap-x-2 gap-y-1">
						<AppVersionValue app={app} service={service} />
						{app.appId ? (
							<a
								{...link(
									{
										screen: "app-devices",
										by: "device",
										focusDeviceId: service.deviceId,
									},
									{ scope: { kind: "app", appId: app.appId } },
								)}
								className={LINK}
							>
								{app.name}
							</a>
						) : null}
					</span>
				</Fact>
				<Fact
					label={t("service.requested.mode", "How it runs")}
					note={
						app.mode ? appCopy(t).mode(app.mode, app.name).sentence : undefined
					}
				>
					{modeText}
				</Fact>
			</Facts>
			<details className="text-ui">
				<summary className="cursor-pointer text-xs text-muted-foreground">
					{t(
						"service.requested.advanced",
						"Advanced: request number and deployment ID",
					)}
				</summary>
				<Facts className="mt-2">
					<Fact
						label={t("service.requested.intent", "Start/stop request")}
						note={t(
							"service.requested.intentNote",
							"Increases each time someone starts, stops or scales it.",
						)}
					>
						<span className="font-mono tabular-nums">
							{placement ? `#${placement.intent_revision}` : "–"}
						</span>
					</Fact>
					<Fact
						label={t("service.requested.deployment", "Deployment ID")}
						note={t(
							"service.requested.deploymentNote",
							"Permanent; cloud approvals are tied to it.",
						)}
					>
						<IdRef
							id={service.deploymentId}
							copyLabel={t(
								"service.requested.copyDeployment",
								"Copy deployment ID",
							)}
						/>
					</Fact>
				</Facts>
			</details>
		</Block>
	);
}

const secretState = (item: ActivityItem) =>
	item.state === "done"
		? "completed"
		: item.state === "failed"
			? "failed"
			: "pending";

function SecretWrites({
	deviceId,
	serviceId,
}: Readonly<{ deviceId: string; serviceId: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const writes = useActivity({ deviceId, serviceId, kind: "secret_write" });
	const waiting = writes.inProgress.length + writes.noReply.length > 0;
	return (
		<Block
			icon={KeyRound}
			title={t("service.secrets.title", "Secret writes")}
			stamp={
				waiting ? (
					<FreshnessStamp
						source="live"
						age="live"
						text={t("service.secrets.checking", "checking every 2 s")}
					/>
				) : (
					<FreshnessStamp
						source="local"
						age="current"
						text={t("service.secrets.tracked", "tracked here")}
					/>
				)
			}
			foot={t(
				"service.secrets.foot",
				"New secret values and access tokens go to the device encrypted and are never readable afterwards. Only writes sent from this computer are tracked here.",
			)}
		>
			{writes.items.length ? (
				<ul className="flex flex-col">
					{writes.items.map((item) => {
						const state = secretState(item);
						return (
							<li
								key={item.id}
								data-secret={state}
								className="flex flex-wrap items-center gap-x-2.5 gap-y-1 border-t border-hairline py-2 text-ui first:border-t-0 first:pt-0"
							>
								<b className="font-semibold">
									{item.resume?.type === "secret"
										? item.resume.name
										: t("service.secrets.unnamed", "A secret value")}
								</b>
								<StatusChip
									tone={
										state === "completed"
											? "good"
											: state === "failed"
												? "critical"
												: "info"
									}
								>
									{enumLabel(t, "secretWrite", state)}
								</StatusChip>
								<span className="text-xs text-muted-foreground">
									{t("service.secrets.sent", "sent {{time}}", {
										time: time.clock(item.startedAt / 1000),
									})}
									{item.finishedAt
										? ` · ${t("service.secrets.finished", "finished {{time}}", {
												time: time.clock(item.finishedAt / 1000),
											})}`
										: ""}
								</span>
							</li>
						);
					})}
				</ul>
			) : (
				<p className="text-ui text-muted-foreground">
					{t("service.secrets.none", "None pending.")}
				</p>
			)}
		</Block>
	);
}

/** The service answers web requests, so it has a service page: its settings say so, or one of its events is served. */
function hasServicePage(
	service: ServiceView,
	app: ServiceApp,
	diagnosis: ServiceDiagnosis,
): boolean {
	if (diagnosis.endpoint) return !!diagnosis.endpoint.host;
	const known = appEventRows(app);
	return (service.events ?? []).some(
		(event) => known.get(event.event_id)?.eligibility.kind === "served",
	);
}

/** The service's quick actions and forms with Run now…, and the runs this computer started. */
function PersonStarted({
	service,
	app,
	diagnosis,
}: Readonly<{
	service: ServiceView;
	app: ServiceApp;
	diagnosis: ServiceDiagnosis;
}>) {
	const openRunNow = useOverlayStore((store) => store.openRunNow);
	const { deviceId, serviceId } = service;
	return (
		<>
			<ActionsBlock
				service={service}
				app={app}
				hasPage={hasServicePage(service, app, diagnosis)}
				onRunNow={(eventId) => openRunNow({ deviceId, serviceId, eventId })}
			/>
			<RunsYouStarted
				deviceId={deviceId}
				serviceId={serviceId}
				events={app.events}
			/>
		</>
	);
}

/** SPEC §5.3 Status: diagnosis, the current update, requested vs actual, update history, secret writes, schedules, bots, actions and forms, and instances. */
export function ServiceStatusTab({
	device,
	service,
	app,
	placement,
	rollout,
	rollouts,
	diagnosis,
	diagnostics,
}: Readonly<{
	device: DeviceViewModel;
	service: ServiceView;
	app: ServiceApp;
	placement: PlacementStatusPlus | undefined;
	/** The update shown as current, if any. */
	rollout: DeploymentRolloutStatus | undefined;
	rollouts: ServiceRollouts;
	diagnosis: ServiceDiagnosis;
	diagnostics: InstanceDiagnostics;
}>) {
	const deviceId = device.row.device_id;
	const name = deviceName(device.row);
	const liveOpen =
		device.live.kind === "live" || device.live.kind === "renewing";
	const requested = (
		<RequestedVsActual
			device={device}
			service={service}
			app={app}
			placement={placement}
		/>
	);
	return (
		<div
			data-service-status=""
			className={`flex min-w-0 flex-col gap-4 ${service.freshness.age === "locked" ? LOCKED_DATA_CLASS : ""}`}
		>
			<div className="grid min-w-0 gap-4 @min-[1000px]/devices:grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)]">
				<div className="flex min-w-0 flex-col gap-4">
					<DiagnosisBlock
						device={device}
						service={service}
						rollout={rollout}
						diagnosis={diagnosis}
						diagnostics={diagnostics}
						hasSchedules={scheduleRows(service, app).length > 0}
						hasBots={botRows(service, app).length > 0}
					/>
					{rollout ? (
						<CurrentUpdate
							deviceId={deviceId}
							deviceName={name}
							service={service}
							rollout={rollout}
							rollouts={rollouts}
						/>
					) : null}
					<UpdateResults deviceId={deviceId} serviceId={service.serviceId} />
					{rollout ? null : requested}
				</div>
				<div className="flex min-w-0 flex-col gap-4">
					{rollout ? requested : null}
					<UpdateHistory
						deviceName={name}
						service={service}
						rollouts={rollouts}
						live={liveOpen}
						offline={device.presence.kind === "offline"}
					/>
					<SecretWrites deviceId={deviceId} serviceId={service.serviceId} />
				</div>
			</div>
			<SchedulesBlock device={device} service={service} app={app} />
			<BotsBlock device={device} service={service} app={app} />
			<PersonStarted service={service} app={app} diagnosis={diagnosis} />
			<InstancesTable
				deviceId={deviceId}
				service={service}
				placement={placement}
				rollout={rollout}
				diagnostics={diagnostics}
			/>
		</div>
	);
}
