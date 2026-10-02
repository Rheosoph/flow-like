"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ListFilter,
	LoaderCircle,
	LockOpen,
	Server,
	SquareX,
	Stethoscope,
	WifiOff,
} from "lucide-react";
import { type ReactNode, useCallback, useMemo } from "react";
import type { DeviceTab } from "../../../../lib/device-management/model/types";
import { liveErrorCode } from "../../../../lib/device-management/workspace/errors";
import { TabsContent } from "../../../ui/tabs";
import { errorCopy } from "../copy/error-copy";
import { useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { DvButton } from "../primitives/dv-button";
import { IdRef } from "../primitives/id-ref";
import { LockedDataBanner, StateView } from "../primitives/state-view";
import { cx } from "../primitives/tone";
import { type UnderlineTab, UnderlineTabs } from "../primitives/underline-tabs";
import { useDevicesRoute } from "../routing/use-devices-route";
import type { ScreenProps } from "../screen-props";
import { useAppNames } from "../shell/attention-popover";
import {
	useAppView,
	useLiveSession,
	useOverlay,
	usePreflight,
} from "../workspace";
import { DeviceAccessTab } from "./access-tab";
import { DeviceActivityTab } from "./activity-tab";
import { DeviceCertificatesTab } from "./certificates-tab";
import { DeviceHeader } from "./device-header";
import { DeviceVerdict } from "./device-verdict";
import { DeviceKeysTab } from "./keys-tab";
import { DeviceMetricsTab } from "./metrics-tab";
import { DeviceOverviewTab } from "./overview-tab";
import { DeviceServicesTab } from "./services-tab";
import { DeviceSettingsTab } from "./settings-tab";
import {
	type DevicePage,
	scopedApp,
	tabCount,
	tabOfAttention,
	useDevicePage,
	usePersonName,
} from "./use-device-page";

const APP_TABS = new Set<DeviceTab>(["overview", "services"]);

/** The app's base style gives every `<p>` a 28 px leading; here a `<p>` without a leading of its own follows its container (zero specificity, so any text utility still wins). */
const PLAIN_PARAGRAPHS = "[:where(&_p)]:leading-[inherit]";

const hasUsableKeys = (page: DevicePage) =>
	!page.revoked &&
	page.view.keys.state !== "none" &&
	page.view.keys.state !== "stale";

function WholeDeviceLabel({ label }: Readonly<{ label: string }>) {
	const { t } = useTranslation("devices");
	return (
		<span className="inline-flex items-center gap-1">
			{label}
			<Server aria-hidden className="size-3 text-muted-foreground" />
			<span className="sr-only">
				{t("device.tabs.wholeDevice", ", whole device")}
			</span>
		</span>
	);
}

function useDeviceTabs(page: DevicePage): UnderlineTab<DeviceTab>[] {
	const { t } = useTranslation("devices");
	return useMemo(() => {
		const labels: Record<DeviceTab, { label: string; short?: string }> = {
			overview: { label: t("device.tabs.overview", "Overview") },
			services: { label: t("device.tabs.services", "Services") },
			activity: {
				label: t("device.tabs.activity", "Activity & logs"),
				short: t("device.tabs.activityShort", "Activity"),
			},
			metrics: { label: t("device.tabs.metrics", "Metrics") },
			certificates: {
				label: t("device.tabs.certificates", "Certificates"),
				short: t("device.tabs.certificatesShort", "Certs"),
			},
			access: { label: t("device.tabs.access", "Access") },
			keys: { label: t("device.tabs.keys", "Keys") },
			settings: {
				label: t("device.tabs.settings", "Device settings"),
				short: t("device.tabs.settingsShort", "Settings"),
			},
		};
		return page.tabs.map((id) => {
			const scoped = APP_TABS.has(id);
			const items = scoped ? page.appAttention : page.attention;
			const mine =
				id === "overview"
					? items
					: items.filter((item) => tabOfAttention(item.key) === id);
			const count = tabCount(mine);
			const { label, short } = labels[id];
			const whole = !!page.app && !scoped;
			return {
				value: id,
				label: whole ? <WholeDeviceLabel label={label} /> : label,
				...(short
					? {
							shortLabel: whole ? <WholeDeviceLabel label={short} /> : short,
						}
					: {}),
				...(count
					? {
							count: {
								...count,
								label: t("device.tabs.badge", {
									count: count.count,
									defaultValue_one: "{{count, number}} item needs you",
									defaultValue_other: "{{count, number}} items need you",
								}),
							},
						}
					: {}),
			};
		});
	}, [t, page.tabs, page.attention, page.appAttention, page.app]);
}

function IdentityBanner({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const overlay = useOverlay();
	const identity = page.identity;
	if (!identity) return null;
	return (
		<Banner
			tone="critical"
			title={t(
				"device.identity.title",
				"The hub reports different keys for {{device}} than the ones you trusted on {{date}}.",
				{ device: page.name, date: time.at(identity.pinnedAt) },
			)}
			actions={
				<DvButton size="sm" onClick={() => overlay.openUnlock(page.deviceId)}>
					{t("device.identity.review", "Review identity…")}
				</DvButton>
			}
		>
			<p className="text-ui">
				{t(
					"device.identity.text",
					"The device may have been set up again, or someone may be impersonating it. Management is blocked: no commands, deploys or access changes until you confirm the identity. Revoking still works, because it happens at the hub.",
				)}
			</p>
			<p className="mt-1.5 flex flex-wrap items-center gap-x-4 gap-y-1 text-ui">
				<IdRef
					id={identity.fingerprint}
					group4
					label={t("device.identity.trusted", "Trusted here")}
					copyLabel={t(
						"device.identity.copyTrusted",
						"Copy trusted fingerprint",
					)}
				/>
				<IdRef
					id={identity.reported}
					group4
					label={t("device.identity.reported", "Hub reports")}
					copyLabel={t(
						"device.identity.copyReported",
						"Copy reported fingerprint",
					)}
				/>
			</p>
		</Banner>
	);
}

function LiveBanner({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	const live = useLiveSession(page.deviceId);
	const state = live.state;
	if (page.view.keys.state !== "unlocked") return null;
	if (state.kind === "reconnecting" || state.kind === "unreachable")
		return (
			<Banner
				tone="info"
				icon={LoaderCircle}
				title={t(
					"device.live.reconnecting",
					"Reconnecting… keys still unlocked.",
				)}
			>
				{t(
					"device.live.reconnectingText",
					"The live connection to {{device}} dropped. The app retries every few seconds. Encrypted status and the hub's data still work; commands wait until the connection is back.",
					{ device: page.name },
				)}
			</Banner>
		);
	if (state.kind !== "failed") return null;
	return (
		<Banner
			tone="warning"
			title={t("device.live.failed", "The live connection failed.")}
			actions={
				<>
					<DvButton size="sm" onClick={() => void live.retry()}>
						{t("device.live.retry", "Try again")}
					</DvButton>
					<DvButton
						size="sm"
						variant="ghost"
						icon={Stethoscope}
						onClick={() => overlay.openDiagnose(page.deviceId)}
					>
						{t("device.live.diagnose", "Diagnose")}
					</DvButton>
				</>
			}
		>
			{errorCopy(t, liveErrorCode(state.cause), { device: page.name })}{" "}
			{t(
				"device.live.failedText",
				"Encrypted status and the hub's data still work; commands need the live connection.",
			)}
		</Banner>
	);
}

function PresenceBanner({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const overlay = useOverlay();
	const ownerName = usePersonName(page.view.row.owner_id, page.consentOnly);
	const { presence } = page.view;
	if (page.revoked)
		return (
			<Banner
				tone="info"
				icon={SquareX}
				title={
					page.consentOnly
						? ownerName
							? t(
									"device.revoked.byOwner",
									"{{device}} was revoked by {{owner}}.",
									{
										device: page.name,
										owner: ownerName,
									},
								)
							: t(
									"device.revoked.byItsOwner",
									"{{device}} was revoked by its owner.",
									{
										device: page.name,
									},
								)
						: t("device.revoked.title", "{{device}} is revoked.", {
								device: page.name,
							})
				}
			>
				{page.consentOnly
					? t(
							"device.revoked.consentText",
							"You see it only because you approve or pay for its cloud access. Revoke the spending limit to stop charges.",
						)
					: page.view.keys.state === "none"
						? t(
								"device.revoked.textNoKeys",
								"The hub refuses it; nobody can manage it from Flow-Like.",
							)
						: t(
								"device.revoked.text",
								"The hub refuses it; nobody can manage it from Flow-Like. Its keys are still stored on this computer, and you can delete them.",
							)}
			</Banner>
		);
	if (presence.kind !== "offline") return null;
	return (
		<Banner
			tone="warning"
			icon={WifiOff}
			title={t(
				"device.offline.title",
				"{{device}} is offline. Everything below is its last known state.",
				{ device: page.name },
			)}
			actions={
				<DvButton
					size="sm"
					icon={Stethoscope}
					onClick={() => overlay.openDiagnose(page.deviceId)}
				>
					{t("device.offline.diagnose", "Diagnose")}
				</DvButton>
			}
		>
			{presence.since === undefined
				? t(
						"device.offline.textNoTime",
						"Actions that need a live connection are disabled until it checks in again.",
					)
				: t(
						"device.offline.text",
						"Its last check-in was {{time}} ({{ago}}). Actions that need a live connection are disabled until it checks in again.",
						{
							time: time.at(presence.since),
							ago: time.ago(presence.since, "long"),
						},
					)}
		</Banner>
	);
}

function DeviceBanners({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	return (
		<>
			<IdentityBanner page={page} />
			{page.lockedRows && !page.services && !page.identity ? (
				<LockedDataBanner
					readAt={page.lockedRows.readAt}
					actions={
						<DvButton
							size="sm"
							icon={LockOpen}
							onClick={() =>
								overlay.openUnlock(page.deviceId, { connectLive: true })
							}
						>
							{t("device.locked.unlock", "Unlock…")}
						</DvButton>
					}
				/>
			) : null}
			<LiveBanner page={page} />
			<PresenceBanner page={page} />
		</>
	);
}

/** APP §1.10: what the app filter covers on this page. */
function ScopeNote({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	if (!page.app) return null;
	return (
		<p
			data-device-scope-note=""
			className="flex items-start gap-2 text-ui text-muted-foreground"
		>
			<ListFilter aria-hidden className="mt-0.5 size-3.5 shrink-0" />
			<span className="max-w-[80ch]">
				<Trans
					t={t}
					i18nKey="device.scopeNote"
					defaults="Showing <1/> on <2/>. Services, and the items and services on Overview, show only this app. The rest, including tabs marked with the device glyph, covers the whole device."
					components={{
						1: <b className="font-semibold text-foreground">{page.app.name}</b>,
						2: <span className="font-mono">{page.name}</span>,
					}}
				/>
			</span>
		</p>
	);
}

function HiddenTabsNote({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	if (!page.consentOnly && !page.revoked) return null;
	return (
		<p className="mt-2 text-xs text-muted-foreground">
			{page.consentOnly
				? t(
						"device.tabs.consentOnly",
						"Only cloud approvals are shown: you have no access to this device itself.",
					)
				: t(
						"device.tabs.revoked",
						"Services, activity, metrics, certificates and device settings aren't shown: revoked devices aren't read.",
					)}
		</p>
	);
}

function DevicePageView({
	route,
	scope,
	page,
}: Readonly<ScreenProps & { page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const appName = useAppNames();
	const { deviceId } = page;
	const requested = route.screen === "device" ? route.tab : undefined;
	const tab: DeviceTab =
		requested && page.tabs.includes(requested) ? requested : "overview";
	const action = route.screen === "device" ? route.action : undefined;
	const extras = useMemo(() => ({ focusDeviceIds: [deviceId] }), [deviceId]);
	const appRead = useAppView(page.app?.id, extras);
	const app = page.app ? appRead : null;
	// The pre-flight is what notices a changed identity before anyone types a password.
	usePreflight(hasUsableKeys(page) ? deviceId : undefined);
	const tabs = useDeviceTabs(page);
	const openTab = useCallback(
		(next: DeviceTab) => navigate({ screen: "device", deviceId, tab: next }),
		[navigate, deviceId],
	);
	const scoped = scopedApp(page);
	const tabProps = { route, scope, deviceId };
	const panels: Record<DeviceTab, () => ReactNode> = {
		overview: () => <DeviceOverviewTab page={page} app={app} onTab={openTab} />,
		services: () => <DeviceServicesTab page={page} app={app} />,
		activity: () => <DeviceActivityTab {...tabProps} />,
		metrics: () => <DeviceMetricsTab {...tabProps} />,
		certificates: () => <DeviceCertificatesTab {...tabProps} />,
		access: () => <DeviceAccessTab {...tabProps} />,
		keys: () => <DeviceKeysTab {...tabProps} />,
		settings: () => <DeviceSettingsTab page={page} action={action} />,
	};
	return (
		<div
			data-device-page={deviceId}
			className={cx("flex min-w-0 flex-col gap-4", PLAIN_PARAGRAPHS)}
		>
			<DeviceHeader
				page={page}
				app={app}
				onOpenKeys={() => openTab("keys")}
				onRevoke={() =>
					navigate({
						screen: "device",
						deviceId,
						tab: "settings",
						action: "revoke",
					})
				}
			/>
			<ScopeNote page={page} />
			<DeviceBanners page={page} />
			<DeviceVerdict
				page={page}
				app={app}
				scopedAppName={scoped ? (appName(scoped) ?? undefined) : undefined}
			/>
			<UnderlineTabs
				value={tab}
				onValueChange={openTab}
				sticky
				label={t("device.tabs.label", "Device sections")}
				tabs={tabs}
			>
				<HiddenTabsNote page={page} />
				<TabsContent value={tab} className="mt-4 min-w-0 outline-none">
					{panels[tab]()}
				</TabsContent>
			</UnderlineTabs>
		</div>
	);
}

/** N2 Device (SPEC §5.2): header, verdict and the eight tabs of one device. */
export function DeviceScreen({
	route,
	scope,
	deviceId,
}: Readonly<ScreenProps & { deviceId: string }>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const read = useDevicePage(deviceId, scope);
	if (read.state === "loading")
		return (
			<StateView
				kind="loading"
				rows={6}
				title={t("device.loading", "Loading this device…")}
			/>
		);
	if (read.state === "unknown")
		return (
			<StateView
				kind="notloaded"
				title={t("device.unknown.title", "This device isn't in your list")}
				text={t(
					"device.unknown.text",
					"It may have been revoked, set up again under another account, or it isn't shared with you any more.",
				)}
				actions={
					<DvButton
						size="sm"
						onClick={() =>
							navigate(
								scope.kind === "app"
									? { screen: "app-devices", by: "device" }
									: { screen: "fleet", view: "devices" },
							)
						}
					>
						{t("device.unknown.back", "Back to devices")}
					</DvButton>
				}
			/>
		);
	return <DevicePageView route={route} scope={scope} page={read.page} />;
}
