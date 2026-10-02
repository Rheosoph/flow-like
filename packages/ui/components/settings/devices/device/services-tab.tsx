"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Boxes,
	Cloud,
	CloudUpload,
	Database,
	Hourglass,
	KeyRound,
	Link2,
	LoaderCircle,
	Lock,
	LockOpen,
	type LucideIcon,
	Play,
	RefreshCw,
	Rocket,
	RotateCw,
	Square,
	SquareX,
	Stethoscope,
} from "lucide-react";
import {
	type MouseEvent,
	type ReactNode,
	useCallback,
	useId,
	useRef,
	useState,
} from "react";
import type { AppServiceRow } from "../../../../lib/device-management/model/app-plan";
import { versionName } from "../../../../lib/device-management/model/app-versions";
import {
	fleetFacts,
	isLastKnown,
} from "../../../../lib/device-management/model/device-view";
import type {
	DevicesRoute,
	DevicesScope,
	GateReason,
	GateResult,
	PlacementConfigFacts,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type {
	ActivityItem,
	ServiceSummary,
} from "../../../../lib/device-management/workspace/types";
import { appCopy } from "../copy/app-copy";
import { formatMoney } from "../copy/attention-copy";
import { enumLabel } from "../copy/enum-labels";
import { DriftChip, MODE_ICON, VersionCell } from "../primitives/app-chips";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { DvTable } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import {
	type Gate,
	GateInline,
	GateNotice,
	GatedAction,
} from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { InlineConfirmRow } from "../primitives/inline-confirm";
import { InlineResult } from "../primitives/inline-result";
import type { DesiredRun, ObservedRun } from "../primitives/requested-actual";
import { ServiceRow, ServiceRowHead } from "../primitives/service-row";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { type ChipTone, cx } from "../primitives/tone";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import {
	type NavigateOptions,
	useDevicesRoute,
	useRouteLink,
} from "../routing/use-devices-route";
import { stampOf, useAppNames } from "../shell/attention-popover";
import { plainClick } from "../shell/rail-row";
import {
	type AppViewRead,
	type ServiceCommand,
	type ServiceCommandId,
	useActivity,
	useAppView,
	useAttentionState,
	useDeviceWorkspace,
	useGate,
	useInlineResults,
	useLiveInspection,
	useLiveSession,
	useOverlay,
	useResourceSummary,
	useServiceCommands,
	useWidthBucket,
} from "../workspace";
import { DeviceAppBlock } from "./app-on-device";
import { DeployAction, GateFix, useDeployTarget } from "./device-header";
import {
	type DevicePage,
	capabilitiesFor,
	gateView,
	scopedApp,
	servicesOfApp,
} from "./use-device-page";

const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";

/** R11: a device with many services shows this many, then "Show more". */
const ROWS_STEP = 50;
/** SPEC §5.2 column plan, with room for "0 of 1 ready" in mono and the update chip. */
const SERVICE_COLS = ["24%", "17%", "12%", "15%", "15%", "17%"] as const;
/** Below the wide bucket (an app's settings, a narrow window) the state and update cells need a larger share. */
const SERVICE_COLS_MEDIUM = ["22%", "19%", "13%", "15%", "16%", "15%"] as const;

const OBSERVED = new Set<string>([
	"unknown",
	"starting",
	"running",
	"stopping",
	"stopped",
	"backoff",
	"failed",
	"removed",
]);

export const desiredRun = (value: string): DesiredRun =>
	value === "stopped" ? "stopped" : "running";
export const observedRun = (value: string): ObservedRun =>
	OBSERVED.has(value) ? (value as ObservedRun) : "unknown";

/* Why a device's decrypted data can't be shown (R6: never "empty"). */

export type DataWhat = "services" | "resources" | "app";

/** "Your access: View status, Read logs · App Invoice AI" for someone the device is shared with. */
function useAccessLine(page: DevicePage): string | undefined {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const appName = useAppNames();
	const scoped = page.app?.id ?? scopedApp(page) ?? undefined;
	const caps = capabilitiesFor(page, scoped) ?? [];
	if (page.owner || !caps.length) return undefined;
	const permissions = new Intl.ListFormat(time.locale, {
		type: "conjunction",
	}).format(caps.map((cap) => enumLabel(t, "capability", cap)));
	const app = scoped ? (appName(scoped) ?? page.app?.name) : undefined;
	return app
		? t(
				"device.state.yourAccessApp",
				"Your access: {{permissions}} · App {{app}}",
				{ permissions, app },
			)
		: t("device.state.yourAccess", "Your access: {{permissions}}", {
				permissions,
			});
}

function LockedState({
	page,
	what,
}: Readonly<{ page: DevicePage; what: DataWhat }>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	const have = useAccessLine(page);
	const device = page.name;
	const titles: Record<DataWhat, string> = {
		services: t(
			"device.state.lockedServices",
			"Unlock {{device}} to see its services.",
			{ device },
		),
		resources: t(
			"device.state.lockedResources",
			"Unlock {{device}} to see its resources.",
			{ device },
		),
		app: t(
			"device.state.lockedApp",
			"Unlock {{device}} to see which {{app}} services run there.",
			{ device, app: page.app?.name ?? "" },
		),
	};
	return (
		<GateNotice
			kind="locked"
			title={titles[what]}
			text={t(
				"device.state.lockedText",
				"The hub can't read them; only keys on this computer can.",
			)}
			have={what === "resources" ? undefined : have}
			actions={
				<DvButton
					size="sm"
					icon={LockOpen}
					onClick={() =>
						overlay.openUnlock(page.deviceId, { connectLive: true })
					}
				>
					{t("device.state.unlock", "Unlock…")}
				</DvButton>
			}
		/>
	);
}

function NoKeysState({
	page,
	what,
}: Readonly<{ page: DevicePage; what: DataWhat }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const texts: Record<DataWhat, string> = {
		services: t(
			"device.state.noKeysServices",
			"The hub can't read its services; only keys on a computer can.",
		),
		resources: t(
			"device.state.noKeysResources",
			"The hub can't read its resources; only keys on a computer can.",
		),
		app: t(
			"device.state.noKeysApp",
			"The hub can't read which {{app}} services run there; only keys on a computer can.",
			{ app: page.app?.name ?? "" },
		),
	};
	return (
		<GateNotice
			kind="nokeys"
			title={t(
				"device.state.noKeys",
				"This computer has no keys for {{device}}.",
				{
					device: page.name,
				},
			)}
			text={texts[what]}
			actions={
				<>
					<DvButton asChild size="sm" icon={KeyRound}>
						<a
							{...link(
								{ screen: "keys", focusDeviceId: page.deviceId },
								{ scope: ACCOUNT_SCOPE },
							)}
						>
							{t("device.state.restoreKeys", "Restore keys…")}
						</a>
					</DvButton>
					<DvButton asChild size="sm" variant="ghost">
						<a
							{...link(
								{ screen: "access", tab: "shared", action: "request" },
								{ scope: ACCOUNT_SCOPE },
							)}
						>
							{t("device.state.requestAccess", "Request access")}
						</a>
					</DvButton>
				</>
			}
		/>
	);
}

const NO_ACCESS_CODES = new Set<string>(["access_ended", "session_closed"]);

function ErrorState({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const [busy, setBusy] = useState(false);
	const retry = async () => {
		setBusy(true);
		try {
			await workspace.fleet.refresh(page.deviceId);
		} catch {
			// The reader records the failure; the state below shows it.
		} finally {
			setBusy(false);
		}
	};
	return (
		<StateView
			kind="error"
			title={t("device.state.errorTitle", "Couldn't read its encrypted status")}
			text={t(
				"device.state.errorText",
				"The app tries again on its own. What the hub knows about {{device}} is still shown.",
				{ device: page.name },
			)}
			actions={
				<DvButton
					size="sm"
					icon={RefreshCw}
					busy={busy}
					onClick={() => void retry()}
				>
					{t("device.state.retry", "Try again")}
				</DvButton>
			}
		/>
	);
}

/**
 * The state shown instead of services or resources while they can't be read.
 * `null`: rows are readable (or kept from before the lock) and the caller renders them.
 */
export function DeviceDataState({
	page,
	what,
}: Readonly<{ page: DevicePage; what: DataWhat }>): ReactNode {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	const link = useRouteLink();
	if (page.revoked)
		return (
			<StateView
				kind="notloaded"
				icon={SquareX}
				title={t("device.state.revokedTitle", "Not available")}
				text={t("device.state.revokedText", "Revoked devices aren't read.")}
			/>
		);
	const unavailable = page.unavailable;
	if (!unavailable) return null;
	const reason = unavailable.reason?.code;
	if (reason === "never_reported")
		return (
			<StateView
				kind="never"
				title={t("device.state.neverTitle", "No status yet")}
				text={t(
					"device.state.neverText",
					"{{device}} sends its first encrypted status after its first check-in. Until then only the hub's registry data exists.",
					{ device: page.name },
				)}
				actions={
					<DvButton
						size="sm"
						icon={Stethoscope}
						onClick={() => overlay.openDiagnose(page.deviceId)}
					>
						{t("device.state.diagnose", "Diagnose")}
					</DvButton>
				}
			/>
		);
	if (reason === "no_keys_here") return <NoKeysState page={page} what={what} />;
	if (unavailable.state === "locked") {
		if (page.lockedRows && what !== "app") return null;
		return <LockedState page={page} what={what} />;
	}
	if (unavailable.state === "noaccess")
		return reason && NO_ACCESS_CODES.has(reason) ? (
			<GateNotice
				kind="noaccess"
				title={t(
					"device.state.accessEnded",
					"Your access to {{device}} has ended.",
					{
						device: page.name,
					},
				)}
				text={t(
					"device.state.accessEndedText",
					"Ask the owner to share it with you again.",
				)}
				actions={
					<DvButton asChild size="sm">
						<a
							{...link(
								{ screen: "access", tab: "shared", action: "request" },
								{ scope: ACCOUNT_SCOPE },
							)}
						>
							{t("device.state.requestAccess", "Request access")}
						</a>
					</DvButton>
				}
			/>
		) : (
			<GateNotice
				kind="noaccess"
				title={t(
					"device.state.noStatusAccess",
					"Your access doesn't include View status.",
				)}
				text={t(
					"device.state.noStatusAccessText",
					"Ask the owner for View status to read what runs on {{device}}.",
					{ device: page.name },
				)}
			/>
		);
	if (unavailable.state === "unsupported")
		return (
			<StateView
				kind="unsupported"
				title={t(
					"device.state.unsupportedTitle",
					"This device's agent can't share its status yet",
				)}
				text={t(
					"device.state.unsupportedText",
					"Update the device agent to read its status here.",
				)}
			/>
		);
	if (unavailable.state === "error") return <ErrorState page={page} />;
	return (
		<StateView
			kind="loading"
			rows={3}
			title={t("device.state.reading", "Reading its encrypted status…")}
		/>
	);
}

/* Internal links inside primitives that only take an `href`: one handler on the table routes them. */

export function useLinkDelegate() {
	const { href, navigate } = useDevicesRoute();
	const routes = useRef(new Map<string, () => void>());
	const register = useCallback(
		(route: DevicesRoute, options?: NavigateOptions) => {
			const target = href(route, options?.scope);
			routes.current.set(target, () => navigate(route, options));
			return target;
		},
		[href, navigate],
	);
	const onClick = useCallback((event: MouseEvent<HTMLElement>) => {
		if (!plainClick(event) || !(event.target instanceof Element)) return;
		const anchor = event.target.closest("a[href]");
		const go = routes.current.get(anchor?.getAttribute("href") ?? "");
		if (!go) return;
		event.preventDefault();
		go();
	}, []);
	return { register, onClick };
}

type Register = ReturnType<typeof useLinkDelegate>["register"];

const appScope = (appId: string): DevicesScope => ({ kind: "app", appId });

/* Cells of one service row. */

interface RowContext {
	page: DevicePage;
	service: ServiceView;
	read: AppViewRead;
	register: Register;
}

function appRowOf({ page, service, read }: RowContext) {
	return read.view?.services.find(
		(row) =>
			row.deviceId === page.deviceId && row.serviceId === service.serviceId,
	);
}

function AppLine(context: Readonly<RowContext>) {
	const { page, service, read, register } = context;
	const { t } = useTranslation("devices");
	const appName = useAppNames();
	const row = appRowOf(context);
	const mode = row?.mode ?? service.source ?? read.view?.app.mode ?? null;
	const name = read.view?.app.name ?? appName(service.projectId);
	const Icon = mode ? MODE_ICON[mode] : Boxes;
	return (
		<>
			<span className="flex min-w-0 basis-full flex-wrap items-center gap-x-1 text-xs text-muted-foreground">
				<Icon aria-hidden className="size-3 shrink-0" />
				{name ? (
					<a
						href={register(
							{
								screen: "app-devices",
								by: "device",
								focusDeviceId: page.deviceId,
							},
							{ scope: appScope(service.projectId) },
						)}
						title={t("device.services.whereRuns", "Where {{app}} runs", {
							app: name,
						})}
						className="font-medium text-ink-2 hover:underline"
					>
						{name}
					</a>
				) : (
					<span>{t("device.services.unknownApp", "An app you can't see")}</span>
				)}
				<span>
					{mode
						? t("device.services.modeLine", "· {{mode}}", {
								mode: appCopy(t).mode(mode, name ?? "").chip,
							})
						: t("device.services.modeUnknown", "· how it runs is unknown")}
				</span>
			</span>
			<ServesLine
				events={row?.events ?? service.events}
				names={read.app?.events}
			/>
		</>
	);
}

const SERVES_SHOWN = 2;

/** "Serves A · B +N", or why the events aren't known (BG-A1). */
function ServesLine({
	events,
	names,
}: Readonly<{
	events: ServiceView["events"];
	names: readonly { id: string; name: string }[] | undefined;
}>) {
	const { t } = useTranslation("devices");
	const line = "basis-full text-xs text-muted-foreground";
	if (events === null)
		return (
			<span className={line}>
				{t(
					"device.services.eventsUnknown",
					"Events unknown: the status snapshot has no event list",
				)}
			</span>
		);
	if (!events.length) return null;
	const byId = new Map(names?.map((event) => [event.id, event.name]));
	const known = events.flatMap((event) => byId.get(event.event_id) ?? []);
	if (!known.length)
		return (
			<span className={line}>
				{t("device.services.servesCount", {
					count: events.length,
					defaultValue_one: "Serves {{count, number}} event",
					defaultValue_other: "Serves {{count, number}} events",
				})}
			</span>
		);
	const shown = known.slice(0, SERVES_SHOWN).join(" · ");
	return (
		<span className={line} title={known.join(", ")}>
			{known.length > SERVES_SHOWN
				? t(
						"device.services.servesMore",
						"Serves {{names}} +{{count, number}}",
						{ names: shown, count: known.length - SERVES_SHOWN },
					)
				: t("device.services.serves", "Serves {{names}}", { names: shown })}
		</span>
	);
}

function ServiceBadges({
	page,
	service,
	facts,
}: Readonly<{
	page: DevicePage;
	service: ServiceView;
	facts: PlacementConfigFacts | undefined;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const summary = useResourceSummary();
	const device = summary.data?.devices.find(
		(row) => row.device_id === page.deviceId,
	);
	const approval = device?.approvals.find(
		(row) => row.placement_id === service.serviceId && row.status === "active",
	);
	const billing = approval
		? device?.billing.find((row) => row.grant_id === approval.grant_id)
		: undefined;
	const writes = service.offlineWrites;
	const buffering =
		writes &&
		writes !== "not_loaded" &&
		(facts?.offlineWrites || writes.pending > 0 || writes.quarantined)
			? writes
			: undefined;
	const chips: {
		id: string;
		icon: LucideIcon;
		tone: ChipTone;
		text: string;
		title?: string;
	}[] = [];
	if (approval)
		chips.push({
			id: "cloud",
			icon: Cloud,
			tone: "outline",
			text: billing
				? t(
						"device.services.cloudSpend",
						"Cloud access · {{used}} of {{limit}}",
						{
							used: formatMoney(billing.used_micros, time.locale),
							limit: formatMoney(billing.limit_micros, time.locale),
						},
					)
				: t("device.services.cloud", "Cloud access"),
		});
	if (buffering)
		chips.push({
			id: "writes",
			icon: Database,
			tone:
				buffering.quarantined || buffering.head?.state === "conflict"
					? "warning"
					: "outline",
			text: buffering.quarantined
				? t("device.services.writesNeedYou", "Write buffering · needs you")
				: buffering.pending > 0
					? t("device.services.writesWaiting", {
							count: buffering.pending,
							defaultValue_one:
								"Write buffering · {{count, number}} change waiting",
							defaultValue_other:
								"Write buffering · {{count, number}} changes waiting",
						})
					: t("device.services.writesUpToDate", "Write buffering · up to date"),
		});
	if (facts?.host && facts.port !== undefined)
		chips.push({
			id: "endpoint",
			icon: facts.tlsCertificateId ? Lock : Link2,
			tone: "outline",
			text: `${facts.host}:${facts.port}`,
			title: `${facts.tlsCertificateId ? "https" : "http"}://${facts.host}:${facts.port}`,
		});
	return (
		<>
			{chips.map((chip) => (
				<StatusChip
					key={chip.id}
					tone={chip.tone}
					icon={chip.icon}
					title={chip.title ?? chip.text}
					className={chip.id === "endpoint" ? "font-mono" : undefined}
				>
					{chip.text}
				</StatusChip>
			))}
		</>
	);
}

const DEPLOY_CAPABILITY = new Set<GateReason>([
	"needs_capability",
	"needs_device_scope",
	"unlock_to_check_permissions",
]);
const DESKTOP_ONLY = new Set<GateReason>([
	"desktop_only",
	"desktop_only_offline_app",
]);
const UPDATE_RUNNING = new Set(["validating", "activating", "rolling_back"]);

/** Why "Update to {version}" can't start now; `null` = it can (the wizard reviews the rest). */
function useUpdateBlocker(
	page: DevicePage,
	service: ServiceView,
): (version: string) => string | null {
	const { t } = useTranslation("devices");
	const gate = useGate("update_service", page.deviceId, {
		projectId: service.projectId,
		placementId: service.serviceId,
	});
	const away =
		isLastKnown(service.freshness) || page.view.presence.kind === "offline";
	return (version) =>
		stateBlocker(t, service, away, version) ?? accessBlocker(t, gate, version);
}

/** The service's own state keeps an update from starting: away, or another update is under way. */
function stateBlocker(
	t: DevicesT,
	service: ServiceView,
	away: boolean,
	version: string,
): string | null {
	if (away)
		return t(
			"devices:device.services.updateOffline",
			"Update to {{version}}: when it's back online",
			{ version },
		);
	const state = service.rollout?.state;
	if (state === "staged")
		return t(
			"devices:device.services.updateAfterStaged",
			"Update to {{version}}: after the staged update is activated or discarded",
			{ version },
		);
	return state && UPDATE_RUNNING.has(state)
		? t(
				"devices:device.services.updateAfterRollout",
				"Update to {{version}}: after this update finishes",
				{ version },
			)
		: null;
}

/** The viewer can't start it from here: no Deploy & configure, or the app needs the desktop app. */
function accessBlocker(
	t: DevicesT,
	gate: GateResult,
	version: string,
): string | null {
	if (gate.ok) return null;
	if (DEPLOY_CAPABILITY.has(gate.copy.code))
		return t(
			"devices:device.services.updateNeedsDeploy",
			"Update to {{version}}: needs Deploy & configure",
			{ version },
		);
	return DESKTOP_ONLY.has(gate.copy.code)
		? t(
				"devices:device.services.updateDesktop",
				"Update to {{version}}: from the desktop app",
				{ version },
			)
		: null;
}

const UPLOADING = new Set<ActivityItem["state"]>(["active", "paused"]);

function isUploadOf(item: ActivityItem, service: ServiceView): boolean {
	const { resume, target } = item;
	if (resume?.type !== "transfer" || resume.projectId !== service.projectId)
		return false;
	const elsewhere =
		target.serviceId !== undefined && target.serviceId !== service.serviceId;
	return !elsewhere && UPLOADING.has(item.state);
}

function uploadOf(
	items: readonly ActivityItem[],
	service: ServiceView,
): ActivityItem | undefined {
	return items.find((item) => isUploadOf(item, service));
}

function UploadLine({
	page,
	service,
	read,
	upload,
}: Readonly<{
	page: DevicePage;
	service: ServiceView;
	read: AppViewRead;
	upload: ActivityItem;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const hash =
		upload.resume?.type === "transfer" ? upload.resume.manifestSha256 : "";
	const label =
		read.view?.versions.find((version) => version.hash === hash)?.label ??
		t("device.services.newVersion", "a new version");
	return (
		<span className="block text-xs text-muted-foreground">
			{uploadText(t, upload, label)}
			{upload.state === "paused" ? (
				<>
					{" "}
					<a
						{...link(deployRoute(page, service, "copy_upload"))}
						title={t(
							"device.services.resumeTitle",
							"Resume the upload where it stopped. Nothing changes on the device until it finishes and you apply it.",
						)}
						className={LINK}
					>
						{t("device.services.resume", "Resume upload…")}
					</a>
				</>
			) : null}
		</span>
	);
}

/** The deploy wizard for this service: an update when the page already names the app. */
function deployRoute(
	page: DevicePage,
	service: ServiceView,
	step?: "copy_upload",
): DevicesRoute {
	return {
		screen: "deploy",
		deviceIds: [page.deviceId],
		serviceId: service.serviceId,
		...(page.app ? { mode: "update" as const } : { appId: service.projectId }),
		...(step ? { step } : {}),
	};
}

function uploadText(
	t: DevicesT,
	upload: ActivityItem,
	version: string,
): string {
	const paused = upload.state === "paused";
	const progress =
		upload.progress && upload.progress !== "indeterminate"
			? upload.progress
			: undefined;
	if (!progress)
		return paused
			? t(
					"devices:device.services.uploadPausedNoCount",
					"Uploading {{version}} · paused",
					{ version },
				)
			: t("devices:device.services.uploadNoCount", "Uploading {{version}}", {
					version,
				});
	const values = { version, done: progress.done, total: progress.total };
	return paused
		? t(
				"devices:device.services.uploadPaused",
				"Uploading {{version}} · paused at {{done, number}} of {{total, number}} files",
				values,
			)
		: t(
				"devices:device.services.uploadActive",
				"Uploading {{version}} · {{done, number}} of {{total, number}} files",
				values,
			);
}

/** The name of the update staged on the device; undefined when it isn't one of the app's known versions. */
function stagedLabelOf(
	row: AppServiceRow | undefined,
	read: AppViewRead,
): string | undefined {
	if (!row?.staged || row.lastChange?.kind !== "update") return undefined;
	const hash = row.lastChange.hash;
	const staged = read.view?.versions.find((version) => version.hash === hash);
	return staged ? versionName(staged) : undefined;
}

/**
 * Label, short hash and drift (APP §7.5). An older version has no name (the
 * hub keeps no version history), so its hash stands in with the drift; only a
 * version the app's list doesn't know says so in words.
 */
function VersionLabel({
	row,
	service,
	staged,
}: Readonly<{
	row: AppServiceRow | undefined;
	service: ServiceView;
	staged: string | undefined;
}>) {
	const { t } = useTranslation("devices");
	const version = row?.version;
	if (version?.label)
		return (
			<VersionCell
				label={version.label}
				hash={version.hash}
				behind={row?.behind ?? null}
				staged={staged}
			/>
		);
	const hash = version?.hash ?? service.appVersion?.hash;
	return (
		<span className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
			{hash ? (
				<IdRef
					id={hash}
					copyLabel={t("device.services.copyHash", "Copy app version hash")}
				/>
			) : null}
			{version ? (
				<DriftChip behind={row?.behind ?? null} staged={staged} />
			) : (
				<span className="text-xs text-muted-foreground">
					{t("device.services.appVersionUnknown", "App version unknown")}
				</span>
			)}
		</span>
	);
}

/** "Update to v1.5.0…" into the wizard, or the reason it can't start now. */
function UpdateLine({
	page,
	service,
	version,
}: Readonly<{ page: DevicePage; service: ServiceView; version: string }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const blocked = useUpdateBlocker(page, service)(version);
	if (blocked !== null)
		return <span className="text-xs text-muted-foreground">{blocked}</span>;
	return (
		<a
			{...link(deployRoute(page, service))}
			title={t(
				"device.services.updateTitle",
				"Update to {{version}} in the deploy wizard. Nothing changes before you review it.",
				{ version },
			)}
			className={cx(LINK, "self-start text-xs")}
		>
			{t("device.services.update", "Update to {{version}}…", { version })}
		</a>
	);
}

function SettingsLine({ service }: Readonly<{ service: ServiceView }>) {
	const { t } = useTranslation("devices");
	const { latest, applied } = service.settings;
	return (
		<span>
			{t("device.services.settings", "Settings v{{version, number}}", {
				version: latest,
			})}
			{applied !== null && applied !== latest ? (
				<span className="block text-xs text-muted-foreground">
					{t(
						"device.services.settingsApplying",
						"running v{{version, number}} · applying",
						{ version: applied },
					)}
				</span>
			) : null}
		</span>
	);
}

/** The newest version when this service runs an older one of the app's list and no upload is under way. */
function updateTargetOf(
	row: AppServiceRow | undefined,
	read: AppViewRead,
	uploading: boolean,
): string | null {
	const newest = read.view?.howRuns.newest;
	const behind = row?.version ? (row.behind ?? 0) : 0;
	return newest && behind > 0 && !uploading ? versionName(newest) : null;
}

function VersionsCell(context: Readonly<RowContext>) {
	const { page, service, read } = context;
	const { t } = useTranslation("devices");
	const { items } = useActivity({ deviceId: page.deviceId, kind: "upload" });
	const row = appRowOf(context);
	const upload = uploadOf(items, service);
	const staged = stagedLabelOf(row, read);
	const target = updateTargetOf(row, read, upload !== undefined);
	return (
		<span className="flex min-w-0 flex-col gap-1">
			<VersionLabel row={row} service={service} staged={staged} />
			{row?.staged && !staged ? (
				<StatusChip tone="info" icon={Rocket} className="self-start">
					{t("device.services.updateStaged", "Update staged")}
				</StatusChip>
			) : null}
			{target ? (
				<UpdateLine page={page} service={service} version={target} />
			) : null}
			{upload ? (
				<UploadLine page={page} service={service} read={read} upload={upload} />
			) : null}
			<SettingsLine service={service} />
		</span>
	);
}

const ROLLOUT_LOOK: Record<
	NonNullable<ServiceView["rollout"]>["state"],
	{ tone: ChipTone; icon: LucideIcon; spin?: boolean }
> = {
	staged: { tone: "info", icon: Hourglass },
	validating: { tone: "info", icon: LoaderCircle, spin: true },
	activating: { tone: "info", icon: LoaderCircle, spin: true },
	healthy: { tone: "good", icon: CloudUpload },
	rolling_back: { tone: "warning", icon: LoaderCircle, spin: true },
	rolled_back: { tone: "warning", icon: RotateCw },
	failed: { tone: "critical", icon: SquareX },
	cancelled: { tone: "outline", icon: SquareX },
};

function UpdateCell({ service }: Readonly<{ service: ServiceView }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const rollout = service.rollout;
	if (!rollout)
		return (
			<span className="text-muted-foreground">
				{t("device.services.noUpdate", "No update running")}
			</span>
		);
	const look = ROLLOUT_LOOK[rollout.state];
	const running =
		UPDATE_RUNNING.has(rollout.state) || rollout.state === "staged";
	const from = rollout.base_revision;
	const to = rollout.active_revision ?? service.settings.latest;
	return (
		<>
			<StatusChip
				tone={look.tone}
				icon={look.icon}
				spin={look.spin}
				title={enumLabel(t, "rollout", rollout.state)}
			>
				{enumLabel(t, "rollout", rollout.state)}
			</StatusChip>
			<span className="mt-0.5 block text-xs text-muted-foreground">
				{running && from !== undefined
					? t(
							"device.services.rolloutVersions",
							"safe update · v{{from, number}} → v{{to, number}}",
							{ from, to },
						)
					: rollout.updated_at !== undefined
						? t("device.services.rolloutAt", "{{time}} · v{{to, number}}", {
								time: time.at(rollout.updated_at),
								to,
							})
						: null}
			</span>
		</>
	);
}

const START_STATES = new Set<ServiceView["conv"]>([
	"crash_looping",
	"failed_stopped",
]);
const COMMAND_ICON: Record<"start" | "stop" | "restart", LucideIcon> = {
	start: Play,
	stop: Square,
	restart: RotateCw,
};

function gateOfCommand(
	t: Parameters<typeof gateView>[0],
	time: Parameters<typeof gateView>[1],
	result: GateResult,
): Gate | null {
	return gateView(t, time, result)?.gate ?? null;
}

interface ActionsProps {
	page: DevicePage;
	service: ServiceView;
	openHref: string;
	onConfirm(action: ServiceCommandId | null): void;
}

function ServiceActions({
	page,
	service,
	openHref,
	onConfirm,
}: Readonly<ActionsProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const commands = useServiceCommands(page.deviceId, service.serviceId);
	const results = useInlineResults(commands.resultKey);
	const reasonId = useId();
	const start = service.desired === "stopped" || START_STATES.has(service.conv);
	const shown: ServiceCommand[] = start
		? [commands.start]
		: [commands.restart, commands.stop];
	const labels: Record<"start" | "stop" | "restart", string> = {
		start: t("device.services.start", "Start"),
		stop: t("device.services.stop", "Stop…"),
		restart: t("device.services.restart", "Restart…"),
	};
	const blocked = page.identity
		? {
				kind: "policy" as const,
				reason: t(
					"device.identity.blockedShort",
					"Management is blocked until the identity is confirmed.",
				),
			}
		: null;
	const gates = shown.map(
		(command) => blocked ?? gateOfCommand(t, time, command.gate),
	);
	const firstGate = gates.find((gate) => gate !== null) ?? null;
	return (
		<span className="flex min-w-0 flex-col items-start gap-1">
			<span className="flex flex-wrap items-center gap-1.5">
				{shown.map((command, index) => {
					const id = command.action as "start" | "stop" | "restart";
					return (
						<DvButton
							key={id}
							size="sm"
							icon={COMMAND_ICON[id]}
							busy={command.pending}
							aria-disabled={gates[index] ? true : undefined}
							aria-describedby={gates[index] ? reasonId : undefined}
							title={command.label}
							onClick={() =>
								command.strength === "none"
									? onConfirm(command.action)
									: void command.run()
							}
						>
							{labels[id]}
						</DvButton>
					);
				})}
				<DvButton asChild size="sm" variant="ghost">
					<a href={openHref}>{t("device.services.open", "Open")}</a>
				</DvButton>
			</span>
			{firstGate ? (
				<GateInline kind={firstGate.kind} id={reasonId}>
					{firstGate.reason}
				</GateInline>
			) : service.conv === "crash_looping" ? (
				<span className="text-xs text-muted-foreground">
					{t(
						"device.services.startClears",
						"Start also clears the crash-loop limit",
					)}
				</span>
			) : null}
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</span>
	);
}

function ConfirmRow({
	page,
	service,
	action,
	onDone,
}: Readonly<{
	page: DevicePage;
	service: ServiceView;
	action: ServiceCommandId;
	onDone(): void;
}>) {
	const { t } = useTranslation("devices");
	const commands = useServiceCommands(page.deviceId, service.serviceId);
	const command =
		action === "start"
			? commands.start
			: action === "stop"
				? commands.stop
				: commands.restart;
	return (
		<InlineConfirmRow
			colSpan={SERVICE_COLS.length}
			label={t("device.services.confirmLabel", "Confirm: {{action}}", {
				action: command.label,
			})}
			title={command.title}
			sub={command.sub}
			rows={command.rows}
			confirmLabel={command.label}
			tone={command.tone}
			onCancel={onDone}
			onConfirm={async () => {
				await command.run({ confirmed: true });
				onDone();
			}}
		/>
	);
}

function DeviceServiceRow({
	page,
	service,
	register,
	titled,
}: Readonly<{
	page: DevicePage;
	service: ServiceView;
	register: Register;
	titled: boolean;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const read = useAppView(service.projectId);
	const [confirming, setConfirming] = useState<ServiceCommandId | null>(null);
	const lastKnown = isLastKnown(service.freshness);
	const facts = fleetFacts(input).byId.get(page.deviceId)?.liveInput
		?.placements?.[service.serviceId];
	const openHref = register({
		screen: "service",
		deviceId: page.deviceId,
		serviceId: service.serviceId,
		tab: "status",
	});
	const context: RowContext = { page, service, read, register };
	const restarts = service.diagnostics?.restarts;
	const since =
		lastKnown && service.freshness.at !== undefined
			? t("device.services.lastKnown", "{{state}} · {{ago}}, last known", {
					state: enumLabel(t, "convergence", service.conv),
					ago: time.ago(service.freshness.at),
				})
			: service.conv === "crash_looping" && restarts
				? t(
						"device.services.crashRestarts",
						"Crashing · restarted {{failures, number}} of {{max, number}} times",
						{ failures: restarts.failures, max: restarts.max_restarts },
					)
				: enumLabel(t, "convergence", service.conv);
	return (
		<>
			<ServiceRow
				serviceId={service.serviceId}
				href={openHref}
				pins={titled ? "titled" : true}
				badges={
					<>
						<AppLine {...context} />
						<ServiceBadges page={page} service={service} facts={facts} />
					</>
				}
				state={{
					desired: desiredRun(service.desired),
					observed: observedRun(service.observed),
					conv: service.conv,
					lastKnown,
					sub: since,
				}}
				instances={{
					ready: service.instances.ready,
					requested: service.instances.requested,
					max: service.instances.max,
				}}
				version={<VersionsCell {...context} />}
				update={<UpdateCell service={service} />}
				dim={lastKnown}
				actions={
					<ServiceActions
						page={page}
						service={service}
						openHref={openHref}
						onConfirm={setConfirming}
					/>
				}
			/>
			{confirming ? (
				<ConfirmRow
					page={page}
					service={service}
					action={confirming}
					onDone={() => setConfirming(null)}
				/>
			) : null}
		</>
	);
}

function LockedServiceRow({
	page,
	row,
	readAt,
}: Readonly<{ page: DevicePage; row: ServiceSummary; readAt: number }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const appName = useAppNames();
	const name = appName(row.projectId);
	return (
		<ServiceRow
			serviceId={row.serviceId}
			pins
			dim
			app={name ? { name } : undefined}
			state={{
				desired: desiredRun(row.desired),
				observed: observedRun(row.observed),
				conv: row.conv,
				lastKnown: true,
				sub: t("device.services.lastKnownLocked", "Last known · {{ago}}", {
					ago: time.ago(readAt),
				}),
			}}
			actions={
				<GateInline kind="locked">
					{t(
						"device.services.unlockForCommands",
						"Unlock {{device}} to run commands.",
						{
							device: page.name,
						},
					)}
				</GateInline>
			}
		/>
	);
}

function RefreshStatus({
	page,
	fix,
}: Readonly<{ page: DevicePage; fix: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const live = useLiveSession(page.deviceId);
	const view = gateView(t, time, useGate("refresh_status", page.deviceId));
	const [state, setState] = useState<"idle" | "busy" | "done">("idle");
	const refresh = async () => {
		setState("busy");
		await live.refresh();
		setState("done");
	};
	const button = (
		<DvButton
			icon={RefreshCw}
			busy={state === "busy"}
			onClick={() => void refresh()}
		>
			{t("device.services.refresh", "Refresh status")}
		</DvButton>
	);
	if (view)
		return (
			<span className="inline-flex flex-wrap items-start gap-2">
				<GatedAction gate={view.gate}>{button}</GatedAction>
				{fix ? <GateFix view={view} /> : null}
			</span>
		);
	return button;
}

function ScopedHint({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const appName = useAppNames();
	const scoped = scopedApp(page);
	if (!scoped) return null;
	const name = appName(scoped) ?? t("device.services.oneApp", "one app");
	return (
		<p className="text-xs text-muted-foreground">
			<Trans
				t={t}
				i18nKey="device.services.scopedHint"
				defaults="You can see services in <1/> only. Other services on <2/> aren't shared with you."
				components={{
					1: (
						<a
							{...link(
								{
									screen: "app-devices",
									by: "device",
									focusDeviceId: page.deviceId,
								},
								{ scope: appScope(scoped) },
							)}
							className={LINK}
						>
							{name}
						</a>
					),
					2: <span className="font-mono">{page.name}</span>,
				}}
			/>
		</p>
	);
}

export interface DeviceServicesTabProps {
	page: DevicePage;
	app: AppViewRead | null;
}

/** SPEC §5.2 Services tab; in an app it starts with the "{App} on {device}" block (APP §1.10). */
export function DeviceServicesTab({
	page,
	app,
}: Readonly<DeviceServicesTabProps>) {
	const { t } = useTranslation("devices");
	const deploy = useDeployTarget(page, app);
	const inspection = useLiveInspection(page.deviceId);
	const delegate = useLinkDelegate();
	const bucket = useWidthBucket();
	const [shown, setShown] = useState(ROWS_STEP);
	const state = <DeviceDataState page={page} what="services" />;
	const blocking = page.revoked || (page.unavailable && !page.lockedRows);
	const all = page.services ?? [];
	const list = servicesOfApp(all, page.app?.id);
	const locked = page.services
		? null
		: page.lockedRows
			? {
					readAt: page.lockedRows.readAt,
					rows: servicesOfApp(page.lockedRows.services, page.app?.id),
				}
			: null;
	const others =
		(page.services ?? page.lockedRows?.services ?? []).length -
		(locked ? locked.rows.length : list.length);
	const count = locked ? locked.rows.length : list.length;
	const first = list[0];
	const title = page.app
		? t("device.services.titleApp", "{{app}} services", { app: page.app.name })
		: t("device.services.title", "Services");
	const appBlock = page.app ? <DeviceAppBlock page={page} app={app} /> : null;
	if (page.app && (blocking || count === 0))
		return (
			<div className="flex min-w-0 flex-col gap-4">
				{appBlock}
				<ScopedHint page={page} />
			</div>
		);
	return (
		<div className="flex min-w-0 flex-col gap-4">
			{appBlock}
			<ScopedHint page={page} />
			<Block
				id="services-block"
				icon={Boxes}
				title={title}
				count={blocking ? undefined : count}
				stamp={
					first ? (
						<FreshnessStamp {...stampOf(first.freshness)} />
					) : locked ? (
						<FreshnessStamp
							source="live"
							age="locked"
							observedAt={locked.readAt}
						/>
					) : null
				}
				tools={
					page.revoked ? null : (
						<>
							<DeployAction target={deploy} fix={!blocking} />
							<RefreshStatus page={page} fix={!blocking} />
						</>
					)
				}
				flush={!blocking && count > 0}
				foot={
					page.revoked ? null : (
						<span>
							{page.app && others > 0 && !blocking
								? `${t("device.services.othersHidden", {
										count: others,
										device: page.name,
										defaultValue_one:
											"{{count, number}} service of other apps on {{device}} isn't shown.",
										defaultValue_other:
											"{{count, number}} services of other apps on {{device}} aren't shown.",
									})} `
								: null}
							{t(
								"device.services.foot",
								"Requested is what you asked for; actual is what the device reports. The agent restarts crashed instances on its own, up to 5 times.",
							)}
						</span>
					)
				}
			>
				{inspection?.progress && page.liveOpen ? (
					<output
						data-services-paging=""
						className={cx(
							"flex items-center gap-2 text-xs text-muted-foreground",
							!blocking && count > 0 && "border-b border-hairline px-4 py-2",
						)}
					>
						<LoaderCircle aria-hidden className="size-3.5 animate-spin" />
						{t(
							"device.services.paging",
							"Reading the service list from the device… part {{part, number}} received. The rows below are from the read before.",
							{ part: inspection.progress.pages },
						)}
					</output>
				) : null}
				{blocking ? (
					state
				) : count === 0 ? (
					<StateView
						kind="empty"
						title={t(
							"device.services.emptyTitle",
							"No services on this device yet",
						)}
						text={t(
							"device.services.emptyText",
							"Deploy an app to run it here.",
						)}
						actions={<DeployAction target={deploy} size="sm" />}
					/>
				) : (
					// biome-ignore lint/a11y/useKeyWithClickEvents: delegates clicks on the links inside the table; the links are keyboard reachable
					<div onClick={delegate.onClick}>
						<DvTable
							cols={bucket === "wide" ? SERVICE_COLS : SERVICE_COLS_MEDIUM}
							head={<ServiceRowHead />}
							label={t("device.services.tableLabel", "Services on {{device}}", {
								device: page.name,
							})}
						>
							{locked
								? locked.rows.map((row) => (
										<LockedServiceRow
											key={row.serviceId}
											page={page}
											row={row}
											readAt={locked.readAt}
										/>
									))
								: list
										.slice(0, shown)
										.map((service, index) => (
											<DeviceServiceRow
												key={service.serviceId}
												page={page}
												service={service}
												register={delegate.register}
												titled={index === 0}
											/>
										))}
						</DvTable>
						{!locked && list.length > shown ? (
							<div className="border-t border-hairline px-4 py-2">
								<DvButton
									size="sm"
									onClick={() => setShown((current) => current + ROWS_STEP)}
								>
									{t("device.services.showMore", {
										count: Math.min(ROWS_STEP, list.length - shown),
										defaultValue_one: "Show {{count, number}} more",
										defaultValue_other: "Show {{count, number}} more",
									})}
								</DvButton>
							</div>
						) : null}
					</div>
				)}
			</Block>
		</div>
	);
}
