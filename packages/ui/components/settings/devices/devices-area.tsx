"use client";

import { useTranslation } from "@flow-like/locales";
import { RefreshCw } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import {
	buildServiceViews,
	deviceName,
} from "../../../lib/device-management/model/device-view";
import type {
	DeviceViewModel,
	DevicesRoute,
	DevicesScope,
} from "../../../lib/device-management/model/types";
import { useBackend } from "../../../state/backend-state";
import { useSpotlightStore } from "../../../state/spotlight-state";
import { Skeleton } from "../../ui/skeleton";
import { AreaOverlays } from "./overlays/area-overlays";
import {
	type AreaTime,
	type DevicesT,
	type HubFreshnessState,
	useAreaTime,
	useHubFreshness,
} from "./primitives/area-context";
import { Banner } from "./primitives/banner";
import { ConfirmProvider } from "./primitives/confirm-sheet";
import { DvButton } from "./primitives/dv-button";
import { StateView } from "./primitives/state-view";
import { healthLabel } from "./primitives/status-chip";
import { ACCOUNT_SCOPE } from "./routing/devices-route";
import type { ResolveFleet } from "./routing/resolve-target";
import {
	DevicesRouteBoundary,
	useDevicesRoute,
} from "./routing/use-devices-route";
import { ActivityButton } from "./shell/activity-button";
import { ActivityTray } from "./shell/activity-tray";
import { AreaGate, type AreaGateProps } from "./shell/area-gate";
import {
	type AreaFrame,
	AreaShell,
	PAGE_FILTER_SELECTOR,
} from "./shell/area-shell";
import { AreaTopbar, appCrumbs } from "./shell/area-topbar";
import { AttentionButton } from "./shell/attention-button";
import { ContextBar } from "./shell/context-bar";
import { DataPlaneBar } from "./shell/data-plane-bar";
import { DeviceRail } from "./shell/device-rail";
import { HubPill } from "./shell/hub-pill";
import { KeysButton } from "./shell/keys-button";
import { PlatformBadge } from "./shell/platform-badge";
import { ResolveBanner } from "./shell/resolve-banner";
import { type RailMode, ScreenSwitch, railMode } from "./shell/screen-switch";
import { useAreaHotkeys } from "./shell/use-area-hotkeys";
import {
	type DevicesSpotlightInput,
	type SpotlightService,
	useDevicesSpotlight,
} from "./shell/use-devices-spotlight";
import {
	AreaProvider,
	type DeviceWorkspaceOverrides,
	DeviceWorkspaceProvider,
	type AreaGate as HubGate,
	type ScopeSwitchNotice,
	type WidthBucket,
	type WorkspaceGate,
	useAreaGate,
	useAreaRootRef,
	useAttentionState,
	useDeviceAuth,
	useDeviceRow,
	useDeviceViews,
	useDeviceWorkspace,
	useFleetDeviceStates,
	useHubSupport,
	useKeyChip,
	useLocalSummary,
	useMyAccess,
	useOverlayStore,
	useScopeSwitchNotice,
	useWidthBucket,
} from "./workspace";

const frameOf = (host: DevicesScope["kind"]): AreaFrame =>
	host === "app" ? "card" : "page";

const openSpotlight = () => useSpotlightStore.getState().open();

/** Same height and line as the data-plane bar while no workspace exists to fill it. */
function IdleStatusBar() {
	return (
		<div
			aria-hidden
			className="h-7 shrink-0 border-t border-hairline bg-surface-sunken"
		/>
	);
}

/** Suspense fallback of the route pages: the frame with nothing loaded yet. */
export function DevicesAreaSkeleton({
	scope = "account",
}: Readonly<{ scope?: DevicesScope["kind"] }>) {
	return (
		<AreaShell
			frame={frameOf(scope)}
			topbar={
				<div className="flex h-12 shrink-0 items-center gap-2 border-b border-hairline px-3">
					<Skeleton aria-hidden className="h-5 w-40 bg-muted" />
					<span className="flex-1" />
					<Skeleton aria-hidden className="h-5 w-56 bg-muted" />
				</div>
			}
			statusBar={<IdleStatusBar />}
		>
			<StateView kind="loading" rows={4} />
		</AreaShell>
	);
}

/* Page-level notices. */

type SwitchNotice = NonNullable<ScopeSwitchNotice["notice"]>;

function switchTitle(t: DevicesT, notice: SwitchNotice) {
	if (!notice.to)
		return t(
			"devices:shell.switch.signedOut",
			"Keys locked because you signed out.",
		);
	if (notice.changed.includes("account"))
		return t(
			"devices:shell.switch.account",
			"Keys locked because you switched accounts.",
		);
	if (notice.changed.includes("hub"))
		return t(
			"devices:shell.switch.hub",
			"Keys locked because you switched to another hub.",
		);
	return t(
		"devices:shell.switch.profile",
		"Keys locked because you switched profiles.",
	);
}

/** IA §6.7 S02: the explicit notice after another account, hub or profile took over. */
function ScopeSwitchBanner() {
	const { t } = useTranslation("devices");
	const { notice, dismiss } = useScopeSwitchNotice();
	if (!notice || notice.lockedSessions === 0) return null;
	const dismissLabel = t("shell.switch.dismiss", "Dismiss");
	const text = t(
		"shell.switch.text",
		"Devices unlocked before: {{count, number}}. Keys are kept apart for each account, hub and profile, so unlock again here when you need them.",
		{ count: notice.lockedSessions },
	);
	const action = (
		<DvButton size="sm" onClick={dismiss}>
			{dismissLabel}
		</DvButton>
	);
	return (
		<Banner tone="locked" title={switchTitle(t, notice)} actions={action}>
			<span data-shell="switch-notice">{text}</span>
		</Banner>
	);
}

/** The cause, what is still shown, what keeps working and when the next try is. */
function hubFailingText(t: DevicesT, hub: HubFreshnessState, time: AreaTime) {
	const parts = [hub.reason ?? ""];
	if (hub.dataFrom !== undefined)
		parts.push(
			t("devices:shell.hubFailing.dataFrom", "Showing data from {{time}}.", {
				time: time.clock(hub.dataFrom),
			}),
		);
	parts.push(
		t(
			"devices:shell.hubFailing.stillWorks",
			"Encrypted status and live connections to your devices still work.",
		),
	);
	if (hub.retryAt !== undefined)
		parts.push(
			t("devices:shell.hubFailing.retrying", "Retrying in {{countdown}}.", {
				countdown: time.countdown(hub.retryAt),
			}),
		);
	return parts.filter(Boolean).join(" ");
}

/** R5: the device list couldn't be refreshed; nothing is cleared and the cause is named. */
function HubFailingBanner() {
	const { t } = useTranslation("devices");
	const hub = useHubFreshness();
	const time = useAreaTime();
	if (!hub.failing) return null;
	const title = t("shell.hubFailing.title", "Couldn't refresh from the hub.");
	const retryLabel = t("shell.hubFailing.retry", "Retry now");
	const text = hubFailingText(t, hub, time);
	const retry = hub.onRetry ? (
		<DvButton size="sm" icon={RefreshCw} onClick={hub.onRetry}>
			{retryLabel}
		</DvButton>
	) : undefined;
	return (
		<Banner tone="warning" title={title} actions={retry}>
			<span data-shell="hub-failing">{text}</span>
		</Banner>
	);
}

function ResolveNotice() {
	const { resolveBanner, dismissResolveBanner } = useDevicesRoute();
	return resolveBanner ? (
		<ResolveBanner banner={resolveBanner} onDismiss={dismissResolveBanner} />
	) : null;
}

/* Area gates. */

/** A gate that concerns the account, on every screen. */
function accountGate(gate: HubGate, signIn: (() => void) | undefined) {
	if (gate.kind !== "token_restricted" && gate.kind !== "session_expired")
		return null;
	const props: AreaGateProps = {
		state: { kind: gate.kind === "token_restricted" ? "token" : gate.kind },
		onSignIn: signIn,
	};
	return props;
}

/** A gate that concerns the hub. The hub screen shows that state itself (SPEC §3.12). */
function hubGate(gate: HubGate, route: DevicesRoute) {
	if (route.screen === "hub") return null;
	if (gate.kind === "hub_unreachable") {
		const props: AreaGateProps = {
			state: { kind: "unreachable", code: gate.error?.code },
			hub: gate.host,
			onRetry: gate.retry,
		};
		return props;
	}
	if (gate.kind !== "hub_checking" && gate.kind !== "hub_off") return null;
	const props: AreaGateProps = {
		state: { kind: gate.kind === "hub_off" ? "off" : "checking" },
		hub: gate.host,
	};
	return props;
}

/** Before a workspace exists: signed out, the profile failed, or both are still loading. */
function workspaceGate(gate: WorkspaceGate) {
	const props: AreaGateProps = { state: { kind: "loading" } };
	if (gate.kind === "signed_out") {
		props.state = { kind: "signed_out" };
		props.onSignIn = gate.signIn;
	}
	if (gate.kind === "profile_unavailable") {
		props.state = { kind: "profile_error" };
		props.onRetry = gate.retry;
	}
	return props;
}

/** No signed-in account or no profile yet: the frame stays, without anything that needs a workspace. */
function AccountGateFrame({ gate }: Readonly<{ gate: WorkspaceGate }>) {
	const { t } = useTranslation("devices");
	const { host } = useDevicesRoute();
	const devices = t("shell.crumbs.devices", "Devices");
	const topbar = (
		<AreaTopbar
			rail={null}
			crumbs={[{ label: devices }]}
			onSearch={openSpotlight}
		/>
	);
	return (
		<AreaShell
			frame={frameOf(host)}
			topbar={topbar}
			statusBar={<IdleStatusBar />}
			banners={<ScopeSwitchBanner />}
		>
			<AreaGate {...workspaceGate(gate)} />
		</AreaShell>
	);
}

/* Bindings of the frame. */

function useAppName(appId: string | undefined) {
	const backend = useBackend();
	const meta = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId ?? ""],
		!!appId,
	);
	return appId ? meta.data?.name : undefined;
}

function routeDeviceId(route: DevicesRoute) {
	if (route.screen === "device" || route.screen === "service")
		return route.deviceId;
	return undefined;
}

function routeDeviceIds(route: DevicesRoute) {
	if (route.screen === "deploy") return route.deviceIds;
	const deviceId = routeDeviceId(route);
	return deviceId ? [deviceId] : [];
}

/** App page: "Devices › edge-berlin-01 › invoice-extractor". */
function useAppCrumbs() {
	const { t } = useTranslation("devices");
	const { route } = useDevicesRoute();
	const deviceId = routeDeviceId(route);
	const row = useDeviceRow(deviceId);
	const device = row ? deviceName(row) : (deviceId?.slice(0, 8) ?? "");
	const devices = t("shell.crumbs.devices", "Devices");
	const deploy = t("shell.crumbs.deploy", "Deploy");
	return useMemo(
		() => appCrumbs(route, { devices, device, deploy }),
		[route, devices, device, deploy],
	);
}

function coversWholeDevice(grant: {
	scope: { kind: string };
	capabilities: readonly string[];
}) {
	return grant.scope.kind === "device" && grant.capabilities.includes("status");
}

/** Owner, or a grant that covers the whole device's status (SPEC §3.3). */
function useCanShowWholeDevice(deviceId: string | undefined) {
	const row = useDeviceRow(deviceId);
	const access = useMyAccess(deviceId);
	const fleet = useFleetDeviceStates();
	if (!deviceId || !row) return false;
	if (row.relationship === "owner") return true;
	if (access.data) return access.data.grants.some(coversWholeDevice);
	const policy = fleet[deviceId]?.policy;
	return policy?.myGrant ? coversWholeDevice(policy.myGrant) : false;
}

function AppContextBar({ appId }: Readonly<{ appId: string }>) {
	const { t } = useTranslation("devices");
	const { route } = useDevicesRoute();
	const name = useAppName(appId);
	const canShowWhole = useCanShowWholeDevice(routeDeviceId(route));
	const fallback = t("shell.context.thisApp", "this app");
	return (
		<ContextBar appName={name ?? fallback} canShowWholeDevice={canShowWhole} />
	);
}

/** Only the ids something is known about: an absent entry means "not known", never "none". */
function knownIds(
	ids: readonly string[],
	read: (id: string) => string[] | undefined,
) {
	const known: Record<string, string[]> = {};
	for (const id of ids) {
		const values = read(id);
		if (values) known[id] = values;
	}
	return known;
}

/** Checks the URL against what is loaded: once the device list is there, and again when more becomes known. */
function useResolveRoute() {
	const { route, href, resolveWith } = useDevicesRoute();
	const { input, items, rows } = useAttentionState();
	const { host } = useHubSupport();
	const backend = useBackend();
	const needsApps = route.screen === "deploy" && !!route.appId;
	const apps = useInvoke(
		backend.appState.getApps,
		backend.appState,
		[],
		needsApps,
	);

	const fleet = useMemo<ResolveFleet | undefined>(() => {
		if (!rows.data) return undefined;
		const ids = routeDeviceIds(route);
		return {
			devices: rows.data,
			hub: host,
			vaults: input.local.vaults,
			attention: items,
			services: knownIds(ids, (id) => {
				const services = buildServiceViews(id, input);
				return Array.isArray(services)
					? services.map((service) => service.serviceId)
					: undefined;
			}),
			certificates: knownIds(ids, (id) =>
				input.certInventory[id]?.certificates.map(
					(entry) => entry.certificate_id,
				),
			),
			...(input.pendingSetups
				? {
						pendingEnrollmentIds: new Set(
							input.pendingSetups.map((setup) => setup.enrollmentId),
						),
					}
				: {}),
			...(apps.data
				? { appIds: new Set(apps.data.map(([app]) => app.id)) }
				: {}),
		};
	}, [rows.data, route, input, items, host, apps.data]);

	const latest = useRef(fleet);
	useEffect(() => {
		latest.current = fleet;
	}, [fleet]);
	const known = fleet
		? [
				Object.keys(fleet.services ?? {}).join(","),
				Object.keys(fleet.certificates ?? {}).join(","),
				fleet.pendingEnrollmentIds ? "setups" : "",
				fleet.appIds ? "apps" : "",
			].join("|")
		: undefined;
	const target = href(route);
	// biome-ignore lint/correctness/useExhaustiveDependencies: runs per target and per level of knowledge, not per poll
	useEffect(() => {
		if (latest.current) resolveWith(latest.current);
	}, [target, known, resolveWith]);
}

function useRail(mode: RailMode, routeKey: string) {
	const [open, setOpen] = useState(false);
	const filter = useRef<HTMLInputElement>(null);
	const focusPending = useRef(false);
	// biome-ignore lint/correctness/useExhaustiveDependencies: a navigation closes the overlay
	useEffect(() => setOpen(false), [routeKey, mode]);
	useEffect(() => {
		if (!open || !focusPending.current) return;
		focusPending.current = false;
		const frame = requestAnimationFrame(() => filter.current?.focus());
		return () => cancelAnimationFrame(frame);
	}, [open]);
	const focusFilter = useCallback(() => {
		if (mode === "none") return false;
		if (mode === "overlay" && !open) {
			focusPending.current = true;
			setOpen(true);
			return true;
		}
		filter.current?.focus();
		return true;
	}, [mode, open]);
	return { open, setOpen, filter, focusFilter };
}

type SpotlightObjects = Pick<
	DevicesSpotlightInput,
	"devices" | "services" | "apps"
>;

/** What Spotlight can find: in an app, only that app's services and the devices running them. */
function spotlightObjects(
	views: readonly DeviceViewModel[],
	appNames: ReadonlyMap<string, string>,
	appId: string | undefined,
	note: (view: DeviceViewModel) => string,
): SpotlightObjects {
	const services: SpotlightService[] = views.flatMap((view) =>
		Array.isArray(view.services)
			? view.services
					.filter((service) => !appId || service.projectId === appId)
					.map((service) => ({
						deviceId: view.row.device_id,
						serviceId: service.serviceId,
						deviceName: deviceName(view.row),
						appName: appNames.get(service.projectId),
					}))
			: [],
	);
	const running = new Set(services.map((service) => service.deviceId));
	return {
		devices: views
			.filter(
				(view) =>
					view.row.status !== "revoked" &&
					(!appId || running.has(view.row.device_id)),
			)
			.map((view) => ({
				id: view.row.device_id,
				name: deviceName(view.row),
				note: note(view),
			})),
		services,
		apps: appId ? [] : [...appNames].map(([id, name]) => ({ id, name })),
	};
}

/** Registers the Spotlight group in a leaf: polls re-render only this, and only a changed list re-registers. */
function DevicesSpotlight() {
	const { t } = useTranslation("devices");
	const { scope, navigate } = useDevicesRoute();
	const views = useDeviceViews();
	const { keys } = useDeviceWorkspace();
	const chip = useKeyChip();
	const backend = useBackend();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const openUnlockSeveral = useOverlayStore((state) => state.openUnlockSeveral);
	const appId = scope.kind === "app" ? scope.appId : undefined;
	const anyLocked = chip.sessions.some((session) => session.state === "locked");
	const anyUnlocked = chip.unlockedCount > 0;

	const signature = JSON.stringify(
		spotlightObjects(
			views,
			new Map(
				(apps.data ?? []).map(([app, meta]) => [app.id, meta?.name ?? app.id]),
			),
			appId,
			(view) => healthLabel(t, view.health),
		),
	);
	const input = useMemo<DevicesSpotlightInput>(
		() => ({
			scope: appId ? { kind: "app", appId } : ACCOUNT_SCOPE,
			...(JSON.parse(signature) as SpotlightObjects),
			navigate,
			...(anyLocked ? { onUnlockSeveral: openUnlockSeveral } : {}),
			...(anyUnlocked ? { onLockAll: () => void keys.lockAll() } : {}),
		}),
		[
			appId,
			signature,
			navigate,
			anyLocked,
			anyUnlocked,
			keys,
			openUnlockSeveral,
		],
	);
	useDevicesSpotlight(input);
	return null;
}

function TopbarChrome({ children }: Readonly<{ children?: ReactNode }>) {
	const { scope, route, navigate } = useDevicesRoute();
	const appName = useAppName(scope.kind === "app" ? scope.appId : undefined);
	return (
		<>
			<AttentionButton
				scope={scope}
				route={route}
				onNavigate={navigate}
				appName={appName}
			/>
			<KeysButton scope={scope} onNavigate={navigate} />
			<ActivityButton />
			{children}
		</>
	);
}

/** The frame with a workspace: chrome, hub gates, banners and the screen. */
function WorkspaceFrame() {
	const { host, scope, route, navigate, href } = useDevicesRoute();
	const areaGate = useAreaGate();
	const { signIn } = useDeviceAuth();
	const gate = accountGate(areaGate, signIn) ?? hubGate(areaGate, route);
	const hub = useHubSupport();
	const local = useLocalSummary();
	const wide = useWidthBucket() === "wide";
	const setAreaRoot = useAreaRootRef();
	const crumbs = useAppCrumbs();
	const mode = gate ? "none" : railMode(route, scope, wide);
	const routeKey = href(route);
	const rail = useRail(mode, routeKey);
	const root = useRef<HTMLDivElement | null>(null);
	useResolveRoute();

	const rootRef = useCallback(
		(element: HTMLDivElement | null) => {
			root.current = element;
			setAreaRoot(element);
		},
		[setAreaRoot],
	);
	const { focusFilter } = rail;
	const onFilter = useCallback(() => {
		const page = root.current?.querySelector<HTMLElement>(PAGE_FILTER_SELECTOR);
		if (!page) return focusFilter();
		page.focus();
		return true;
	}, [focusFilter]);
	useAreaHotkeys({ onFilter });

	const deviceId = routeDeviceId(route);
	return (
		<>
			<AreaShell
				frame={frameOf(host)}
				rootRef={rootRef}
				scrollKey={`${route.screen}:${deviceId ?? ""}:${route.screen === "service" ? route.serviceId : ""}`}
				topbar={
					<AreaTopbar
						rail={
							mode === "overlay"
								? {
										open: rail.open,
										onToggle: () => rail.setOpen(!rail.open),
									}
								: null
						}
						crumbs={crumbs}
						onSearch={openSpotlight}
						trailing={
							<>
								<HubPill state={hub.support.state} />
								<PlatformBadge
									platform={local.platform}
									persistence={local.persistence}
								/>
							</>
						}
					>
						<TopbarChrome />
					</AreaTopbar>
				}
				rail={
					mode === "none" ? null : (
						<DeviceRail
							scope={scope}
							route={route}
							onNavigate={navigate}
							mode={mode}
							open={rail.open}
							onOpenChange={rail.setOpen}
							filterRef={rail.filter}
						/>
					)
				}
				tray={<ActivityTray scope={scope} onNavigate={navigate} />}
				statusBar={<DataPlaneBar scope={scope} onNavigate={navigate} />}
				banners={
					<>
						<ScopeSwitchBanner />
						<ResolveNotice />
						{gate ? null : <HubFailingBanner />}
						{gate || scope.kind !== "app" || !deviceId ? null : (
							<AppContextBar appId={scope.appId} />
						)}
					</>
				}
			>
				{gate ? (
					<AreaGate {...gate} />
				) : (
					<ScreenSwitch route={route} scope={scope} />
				)}
			</AreaShell>
			<DevicesSpotlight />
		</>
	);
}

/** Tests and the visual harness: what the workspace provider reads instead of the host, a fixed width bucket, a stopped clock. */
export interface DevicesAreaHarness {
	overrides?: DeviceWorkspaceOverrides;
	widthBucket?: WidthBucket;
	tickMs?: number | false;
}

function AreaRoot({ harness }: Readonly<{ harness?: DevicesAreaHarness }>) {
	const { redirecting, host } = useDevicesRoute();
	const renderGate = useCallback(
		(gate: WorkspaceGate) => <AccountGateFrame gate={gate} />,
		[],
	);
	if (redirecting) return <DevicesAreaSkeleton scope={host} />;
	return (
		<DeviceWorkspaceProvider
			renderGate={renderGate}
			overrides={harness?.overrides}
		>
			<AreaProvider widthBucket={harness?.widthBucket} tickMs={harness?.tickMs}>
				<ConfirmProvider>
					<WorkspaceFrame />
					<AreaOverlays />
				</ConfirmProvider>
			</AreaProvider>
		</DeviceWorkspaceProvider>
	);
}

/**
 * The Devices area in both hosts: `/settings/devices` (`account`) and an app's
 * `/library/config/devices?id=` (`app`). Wrap it in `Suspense`: it reads the URL.
 */
export function DevicesArea({
	scope,
	harness,
}: Readonly<{ scope: DevicesScope["kind"]; harness?: DevicesAreaHarness }>) {
	return (
		<DevicesRouteBoundary host={scope}>
			<AreaRoot harness={harness} />
		</DevicesRouteBoundary>
	);
}
