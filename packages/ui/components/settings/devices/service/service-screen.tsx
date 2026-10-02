"use client";

import { useTranslation } from "@flow-like/locales";
import {
	KeyRound,
	LoaderCircle,
	LockOpen,
	Stethoscope,
	WifiOff,
} from "lucide-react";
import { useCallback, useMemo } from "react";
import { agentSupports } from "../../../../lib/device-management/agent-reads";
import {
	deviceListKnown,
	deviceName,
} from "../../../../lib/device-management/model/device-view";
import {
	type AgentFeatures,
	type AttentionItem,
	type AttentionKey,
	type DeviceViewModel,
	type DevicesRoute,
	type DevicesScope,
	SERVICE_TABS,
	type ServiceTab,
	type ServiceView,
} from "../../../../lib/device-management/model/types";
import { liveErrorCode } from "../../../../lib/device-management/workspace/errors";
import { TabsContent } from "../../../ui/tabs";
import { errorCopy } from "../copy/error-copy";
import { useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { DvButton } from "../primitives/dv-button";
import { GateNotice } from "../primitives/gate-notice";
import { Headline } from "../primitives/headline";
import { RequestedActual } from "../primitives/requested-actual";
import { LockedDataBanner, StateView } from "../primitives/state-view";
import {
	type TabCountTone,
	type UnderlineTab,
	UnderlineTabs,
} from "../primitives/underline-tabs";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import type { ScreenProps, ServiceTabProps } from "../screen-props";
import {
	type ServiceViewRead,
	useAttentionState,
	useOverlay,
	useServiceView,
} from "../workspace";
import { ServiceActivityTab } from "./activity-tab";
import { ServiceCloudTab } from "./cloud-tab";
import { ServiceConfigurationTab } from "./configuration-tab";
import { currentUpdateOf, useServiceRollouts } from "./current-update";
import { useServiceDiagnosis } from "./diagnosis-banner";
import { ServiceEndpointTab } from "./endpoint-tab";
import { type InstanceDiagnostics, usePlacementRow } from "./instances-table";
import { ServiceMetricsTab } from "./metrics-tab";
import { ServiceOfflineTab } from "./offline-tab";
import { ServiceActions } from "./service-actions";
import {
	type ServiceApp,
	ServiceAppContextLine,
	ServiceHeader,
	useServiceApp,
} from "./service-header";
import { ServiceStatusTab } from "./status-tab";

const STATUS_CRITICAL = new Set<AttentionKey>([
	"service_crash_looping",
	"rollout_failed_service_stopped",
]);
const STATUS_WARNING = new Set<AttentionKey>([
	"rollout_rolled_back",
	"rollout_not_applied",
]);

type Badge = NonNullable<UnderlineTab<ServiceTab>["count"]>;

const badge = (count: number, tone: TabCountTone, label: string): Badge => ({
	count,
	tone,
	label,
});

function useTabs(
	service: ServiceView | undefined,
	attention: readonly AttentionItem[],
	queuesNeedYou: number,
	quarantined: boolean,
	cloudInvalid: boolean,
): UnderlineTab<ServiceTab>[] {
	const { t } = useTranslation("devices");
	return useMemo(() => {
		const has = (keys: ReadonlySet<AttentionKey>) =>
			attention.some((item) => keys.has(item.key));
		const crashed =
			service?.conv === "crash_looping" || service?.conv === "failed_stopped";
		const badges: Partial<Record<ServiceTab, Badge>> = {};
		if (crashed || has(STATUS_CRITICAL))
			badges.status = badge(
				1,
				"critical",
				t(
					"service.tabs.statusCritical",
					"The service isn't running as requested",
				),
			);
		else if (has(STATUS_WARNING))
			badges.status = badge(
				1,
				"warning",
				t("service.tabs.statusWarning", "The last update didn't apply"),
			);
		if (queuesNeedYou)
			badges.offline = badge(
				queuesNeedYou,
				"warning",
				t("service.tabs.offlineBadge", {
					count: queuesNeedYou,
					defaultValue_one: "{{count, number}} queue needs you",
					defaultValue_other: "{{count, number}} queues need you",
				}),
			);
		if (cloudInvalid)
			badges.cloud = badge(
				1,
				"critical",
				t("service.tabs.cloudRevoked", "Bound to a revoked approval"),
			);
		else if (quarantined)
			badges.cloud = badge(
				1,
				"warning",
				t(
					"service.tabs.cloudPaused",
					"Buffered changes paused because cloud access changed",
				),
			);
		const labels: Record<ServiceTab, [label: string, short?: string]> = {
			status: [t("service.tabs.status", "Status")],
			activity: [
				t("service.tabs.activity", "Activity & logs"),
				t("service.tabs.activityShort", "Activity"),
			],
			metrics: [t("service.tabs.metrics", "Metrics")],
			configuration: [
				t("service.tabs.configuration", "Configuration"),
				t("service.tabs.configurationShort", "Config"),
			],
			endpoint: [t("service.tabs.endpoint", "Endpoint")],
			cloud: [
				t("service.tabs.cloud", "Cloud access"),
				t("service.tabs.cloudShort", "Cloud"),
			],
			offline: [
				t("service.tabs.offline", "Write buffering"),
				t("service.tabs.offlineShort", "Buffering"),
			],
		};
		return SERVICE_TABS.map((value) => {
			const [label, shortLabel] = labels[value];
			const count = badges[value];
			return {
				value,
				label,
				...(shortLabel ? { shortLabel } : {}),
				...(count ? { count } : {}),
			};
		});
	}, [t, service?.conv, attention, queuesNeedYou, quarantined, cloudInvalid]);
}

/** Why the service's row can't be shown: never "empty" (R6). */
function Unreadable({
	device,
	serviceId,
	read,
}: Readonly<{
	device: DeviceViewModel;
	serviceId: string;
	read: ServiceViewRead;
}>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	const link = useRouteLink();
	const deviceId = device.row.device_id;
	const name = deviceName(device.row);
	const { unavailable } = read;
	const summary = device.keys.lockedSummary;
	const last = summary?.services.find((row) => row.serviceId === serviceId);

	if (!unavailable)
		return (
			<StateView
				kind="notloaded"
				title={t(
					"service.unreadable.missing",
					"No service {{service}} on {{device}}",
					{
						service: serviceId,
						device: name,
					},
				)}
				text={t(
					"service.unreadable.missingText",
					"It may have been removed, or deployed again under another ID.",
				)}
				actions={
					<DvButton asChild size="sm">
						<a {...link({ screen: "device", deviceId, tab: "services" })}>
							{t(
								"service.unreadable.openServices",
								"Open {{device}}'s services",
								{
									device: name,
								},
							)}
						</a>
					</DvButton>
				}
			/>
		);
	if (device.presence.kind === "revoked")
		return (
			<StateView
				kind="notloaded"
				title={t("service.unreadable.revoked", "Revoked devices aren't read.")}
				text={t(
					"service.unreadable.revokedText",
					"Nothing is known about this service since {{device}} was revoked.",
					{ device: name },
				)}
			/>
		);
	if (
		unavailable.reason?.code === "no_keys_here" ||
		device.keys.state === "none"
	)
		return (
			<GateNotice
				kind="nokeys"
				title={t(
					"service.unreadable.noKeys",
					"This computer has no keys for {{device}}.",
					{ device: name },
				)}
				text={t(
					"service.unreadable.noKeysText",
					"The hub can't read its services; only keys on a computer can.",
				)}
				actions={
					<>
						<DvButton asChild size="sm" icon={KeyRound}>
							<a {...link({ screen: "keys", focusDeviceId: deviceId })}>
								{t("service.unreadable.restore", "Restore keys…")}
							</a>
						</DvButton>
						<DvButton asChild size="sm" variant="ghost">
							<a
								{...link({
									screen: "access",
									tab: "shared",
									action: "request",
								})}
							>
								{t("service.unreadable.request", "Request access")}
							</a>
						</DvButton>
					</>
				}
			/>
		);
	if (unavailable.reason?.code === "never_reported")
		return (
			<StateView
				kind="never"
				title={t("service.unreadable.never", "No status yet")}
				text={t(
					"service.unreadable.neverText",
					"{{device}} sends its first encrypted status after its first check-in.",
					{ device: name },
				)}
			/>
		);
	if (unavailable.state === "locked")
		return (
			<div className="flex flex-col gap-3">
				{summary && last ? (
					<>
						<LockedDataBanner readAt={summary.readAt} />
						<p className="text-ui text-muted-foreground">
							<RequestedActual
								desired={last.desired === "stopped" ? "stopped" : "running"}
								observed="unknown"
								conv={last.conv}
							/>
						</p>
					</>
				) : null}
				<GateNotice
					kind="locked"
					title={t(
						"service.unreadable.locked",
						"Unlock {{device}} to see this service.",
						{ device: name },
					)}
					text={t(
						"service.unreadable.lockedText",
						"Its status, instances and updates are encrypted for keys on this computer.",
					)}
					actions={
						<DvButton
							size="sm"
							icon={LockOpen}
							onClick={() => overlay.openUnlock(deviceId)}
						>
							{t("service.unreadable.unlock", "Unlock…")}
						</DvButton>
					}
				/>
			</div>
		);
	if (unavailable.state === "noaccess")
		return (
			<GateNotice
				kind="noaccess"
				title={t(
					"service.unreadable.noAccess",
					"Your access to {{device}} doesn't cover this service.",
					{ device: name },
				)}
				text={t(
					"service.unreadable.noAccessText",
					"Ask the owner to share it with you again.",
				)}
			/>
		);
	if (unavailable.state === "unsupported")
		return (
			<StateView
				kind="unsupported"
				title={t(
					"service.unreadable.unsupported",
					"{{device}} doesn't send encrypted status",
					{ device: name },
				)}
				text={t(
					"service.unreadable.unsupportedText",
					"Connect live to read this service.",
				)}
			/>
		);
	if (unavailable.state === "error")
		return (
			<StateView
				kind="error"
				title={t(
					"service.unreadable.error",
					"The status of {{device}} couldn't be read",
					{ device: name },
				)}
				text={t(
					"service.unreadable.errorText",
					"The app keeps trying. Diagnose shows what fails.",
				)}
				actions={
					<DvButton
						size="sm"
						icon={Stethoscope}
						onClick={() => overlay.openDiagnose(deviceId, serviceId)}
					>
						{t("service.unreadable.diagnose", "Diagnose")}
					</DvButton>
				}
			/>
		);
	return <StateView kind="loading" rows={4} />;
}

function PageBanners({
	device,
	service,
}: Readonly<{ device: DeviceViewModel; service: ServiceView | undefined }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const overlay = useOverlay();
	const deviceId = device.row.device_id;
	const name = deviceName(device.row);
	const { presence, live } = device;
	return (
		<>
			{presence.kind === "offline" && service ? (
				<Banner
					tone="warning"
					icon={WifiOff}
					title={t(
						"service.banner.offline",
						"{{device}} is offline. Everything below is this service's last known state.",
						{ device: name },
					)}
					actions={
						<DvButton
							size="sm"
							icon={Stethoscope}
							onClick={() => overlay.openDiagnose(deviceId, service.serviceId)}
						>
							{t("service.banner.diagnose", "Diagnose")}
						</DvButton>
					}
				>
					{presence.since
						? t(
								"service.banner.offlineText",
								"Its last check-in was {{at}} ({{ago}}). Commands, settings and logs need a live connection and are disabled until it checks in again.",
								{ at: time.at(presence.since), ago: time.ago(presence.since) },
							)
						: t(
								"service.banner.offlineNoTime",
								"Commands, settings and logs need a live connection and are disabled until it checks in again.",
							)}
				</Banner>
			) : null}
			{live.kind === "reconnecting" || live.kind === "unreachable" ? (
				<Banner
					tone="info"
					icon={LoaderCircle}
					title={t(
						"service.banner.reconnecting",
						"Reconnecting… keys still unlocked.",
					)}
				>
					{t(
						"service.banner.reconnectingText",
						"Encrypted status and the hub's data still work; commands wait until the connection is back.",
					)}{" "}
					{errorCopy(t, liveErrorCode(live.cause))}
				</Banner>
			) : null}
		</>
	);
}

function OtherTab(props: Readonly<ServiceTabProps & { tab: ServiceTab }>) {
	const { tab, ...tabProps } = props;
	switch (tab) {
		case "activity":
			return <ServiceActivityTab {...tabProps} />;
		case "metrics":
			return <ServiceMetricsTab {...tabProps} />;
		case "configuration":
			return <ServiceConfigurationTab {...tabProps} />;
		case "endpoint":
			return <ServiceEndpointTab {...tabProps} />;
		case "cloud":
			return <ServiceCloudTab {...tabProps} />;
		default:
			return <ServiceOfflineTab {...tabProps} />;
	}
}

/** Last errors and restart counts: reported, missing on an older agent, or unknown without a live read. */
function diagnosticsOf(
	features: AgentFeatures | undefined,
	rowHasThem: boolean,
) {
	if (rowHasThem) return "reported";
	if (agentSupports(features, "placement_diagnostics")) return "reported";
	if (features) return "old_agent";
	return "not_live";
}

/** Everything the page derives for one service: its app, its updates, the device's own row and the diagnosis. */
function useServicePage(
	scope: DevicesScope,
	device: DeviceViewModel,
	serviceId: string,
	service: ServiceView | undefined,
) {
	const time = useAreaTime();
	const { input } = useAttentionState();
	const deviceId = device.row.device_id;
	const app = useServiceApp(
		service,
		scope.kind === "app" ? scope.appId : undefined,
	);
	const rollouts = useServiceRollouts(deviceId, service);
	const rollout = currentUpdateOf(service, time.nowS);
	const placement = usePlacementRow(deviceId, service);
	const diagnosis = useServiceDiagnosis(
		device,
		serviceId,
		service,
		rollout,
		placement?.offline_writes?.needs_attention,
	);
	const diagnostics: InstanceDiagnostics = diagnosticsOf(
		input.live[deviceId]?.inspection?.value.features,
		service?.diagnostics !== undefined,
	);
	return { app, rollouts, rollout, placement, diagnosis, diagnostics };
}

/** App scope only: where else the app runs, or that this service belongs to another app. */
function AppContextNotice({
	scope,
	deviceId,
	serviceId,
	tab,
	service,
	app,
}: Readonly<{
	scope: DevicesScope;
	deviceId: string;
	serviceId: string;
	tab: ServiceTab;
	service: ServiceView | undefined;
	app: ServiceApp;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { input } = useAttentionState();
	const deviceNames = useCallback(
		(id: string) => {
			const row = input.devices.find((entry) => entry.device_id === id);
			return row ? deviceName(row) : id.slice(0, 8);
		},
		[input.devices],
	);
	if (scope.kind !== "app") return null;
	if (!service || service.projectId === scope.appId)
		return (
			<ServiceAppContextLine
				appId={scope.appId}
				deviceId={deviceId}
				serviceId={serviceId}
				app={app}
				deviceNames={deviceNames}
			/>
		);
	return (
		<Banner tone="info">
			{t(
				"service.context.otherApp",
				"{{service}} runs {{app}}, not the app whose settings you opened.",
				{ service: serviceId, app: app.name },
			)}{" "}
			<a
				{...link(
					{ screen: "service", deviceId, serviceId, tab },
					{ scope: { kind: "app", appId: service.projectId } },
				)}
				className="underline decoration-border-strong underline-offset-2 hover:decoration-current"
			>
				{t("service.context.openOtherApp", "Open it in {{app}}'s settings", {
					app: app.name,
				})}
			</a>
		</Banner>
	);
}

function ServicePage({
	route,
	scope,
	deviceId,
	serviceId,
	device,
	read,
}: Readonly<
	ScreenProps & {
		deviceId: string;
		serviceId: string;
		device: DeviceViewModel;
		read: ServiceViewRead;
	}
>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const { service } = read;
	const tab = routeTab(route);
	const page = useServicePage(scope, device, serviceId, service);
	const { app, diagnosis } = page;
	const tabs = useTabs(
		service,
		diagnosis.attention,
		diagnosis.queues?.needYou ?? 0,
		diagnosis.queues?.quarantined ?? false,
		diagnosis.cloudInvalid,
	);
	const onTab = useCallback(
		(next: ServiceTab) =>
			navigate({ screen: "service", deviceId, serviceId, tab: next }),
		[navigate, deviceId, serviceId],
	);
	const status = service ? (
		<ServiceStatusTab
			device={device}
			service={service}
			app={app}
			placement={page.placement}
			rollout={page.rollout}
			rollouts={page.rollouts}
			diagnosis={diagnosis}
			diagnostics={page.diagnostics}
		/>
	) : (
		<Unreadable device={device} serviceId={serviceId} read={read} />
	);

	return (
		<div data-service-screen="" className="flex min-w-0 flex-col gap-4">
			<ServiceHeader
				scope={scope}
				deviceId={deviceId}
				serviceId={serviceId}
				tab={tab}
				device={device}
				service={service}
				app={app}
				rollout={page.rollout}
			/>
			<AppContextNotice
				scope={scope}
				deviceId={deviceId}
				serviceId={serviceId}
				tab={tab}
				service={service}
				app={app}
			/>
			<PageBanners device={device} service={service} />
			<Headline
				lead={diagnosis.verdict.lead}
				rest={diagnosis.verdict.rest || undefined}
			/>
			{service ? (
				<ServiceActions
					device={device}
					service={service}
					app={app}
					placement={page.placement}
					endpoint={diagnosis.endpoint}
				/>
			) : null}
			<UnderlineTabs
				label={t("service.tabs.label", "Service sections")}
				tabs={tabs}
				value={tab}
				onValueChange={onTab}
				sticky
			>
				<TabsContent value={tab} className="mt-4 min-w-0 outline-none">
					{tab === "status" ? (
						status
					) : (
						<OtherTab
							tab={tab}
							route={route}
							scope={scope}
							deviceId={deviceId}
							serviceId={serviceId}
						/>
					)}
				</TabsContent>
			</UnderlineTabs>
		</div>
	);
}

/** SPEC §5.3 N3: operate, diagnose, reconfigure, expose and fund one service. */
export function ServiceScreen({
	route,
	scope,
	deviceId,
	serviceId,
}: Readonly<ScreenProps & { deviceId: string; serviceId: string }>) {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const read = useServiceView(deviceId, serviceId);
	if (!read.device)
		return deviceListKnown(input) ? (
			<StateView
				kind="notloaded"
				title={t(
					"service.unreadable.noDevice",
					"This device isn't in your list",
				)}
				text={t(
					"service.unreadable.noDeviceText",
					"It may have been removed, or it isn't shared with this account.",
				)}
			/>
		) : (
			<StateView kind="loading" rows={5} />
		);
	return (
		<ServicePage
			route={route}
			scope={scope}
			deviceId={deviceId}
			serviceId={serviceId}
			device={read.device}
			read={read}
		/>
	);
}

function routeTab(route: DevicesRoute): ServiceTab {
	return route.screen === "service" && route.tab ? route.tab : "status";
}
