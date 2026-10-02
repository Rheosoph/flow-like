"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Boxes,
	Check,
	CircleArrowUp,
	CircleCheck,
	CircleSlash,
	Copy,
	Hourglass,
	KeyRound,
	Layers,
	LayoutGrid,
	LoaderCircle,
	LockOpen,
	type LucideIcon,
	OctagonX,
	RefreshCw,
	RotateCcw,
	SlidersHorizontal,
} from "lucide-react";
import { Fragment, type ReactNode, useMemo } from "react";
import type { DeploymentRolloutStatus } from "../../../../lib/device-management/deployment";
import type {
	AppMode,
	AppServiceRow,
	AppView,
} from "../../../../lib/device-management/model/app-plan";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	DeviceViewModel,
	DevicesRoute,
	DevicesScope,
	PlacementEvent,
	ServiceTab,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import { appCopy } from "../copy/app-copy";
import { formatNames } from "../copy/attention-copy";
import { enumLabel } from "../copy/enum-labels";
import { DriftChip, MODE_ICON } from "../primitives/app-chips";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { type Crumb, type ObjectFact, ObjectHeader } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { IdRef } from "../primitives/id-ref";
import { PairedPins } from "../primitives/paired-pins";
import { PresenceGlyph } from "../primitives/presence-glyph";
import {
	type DesiredRun,
	type ObservedRun,
	RequestedActual,
} from "../primitives/requested-actual";
import {
	ConvergenceChip,
	KeyChip,
	PresenceChip,
	StatusChip,
} from "../primitives/status-chip";
import type { ChipTone } from "../primitives/tone";
import { useCopy } from "../primitives/use-copy";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import { stampOf, useAppNames } from "../shell/attention-popover";
import { keyChipOf } from "../shell/keys-popover";
import { useAppView, useCoverage, useOverlay } from "../workspace";
import { rolloutEndedAt, rolloutPhase } from "./current-update";

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

export const desiredRun = (service: ServiceView): DesiredRun =>
	service.desired === "stopped" ? "stopped" : "running";
export const observedRun = (service: ServiceView): ObservedRun =>
	OBSERVED.has(service.observed)
		? (service.observed as ObservedRun)
		: "unknown";

export interface ServedEvent {
	id: string;
	/** From the app's metadata; undefined when the app no longer has the event. */
	name?: string;
	version: string;
	eventType?: string;
}

/** Whether the service runs the versions published now: counted when the app's version list is known, else from its event pins. */
export type ServiceDrift =
	| { kind: "count"; behind: number }
	| { kind: "pins"; behind: boolean }
	| { kind: "unknown" };

export interface ServiceApp {
	appId: string | undefined;
	/** The app's name, or a short stand-in when this account can't read it. */
	name: string;
	mode: AppMode | null;
	view: AppView | undefined;
	/** The app's events and versions: read, still loading, or not readable with this account's role. */
	metadata: "loaded" | "loading" | "unreadable";
	row: AppServiceRow | null;
	/** null = the plane that produced the row carries no event list. */
	events: ServedEvent[] | null;
	drift: ServiceDrift;
	/** Label of the newest version, when the version list is known. */
	newestLabel: string | null;
}

const triple = (value: readonly number[]) => value.join(".");

type PublishedPin = AppView["events"]["rows"][number]["pin"];

const samePin = (pin: NonNullable<PublishedPin>, event: PlacementEvent) =>
	triple(pin.eventVersion) === triple(event.event_version) &&
	triple(pin.boardVersion) === triple(event.board_version);

/** Whether a served event is pinned to an older version than the app publishes now; null when an event's pin isn't known. */
function pinsBehind(
	view: AppView,
	events: readonly PlacementEvent[],
): boolean | null {
	const published = new Map<string, PublishedPin>(
		[...view.events.rows, ...view.events.ineligible].map((row) => [
			row.eventId,
			row.pin,
		]),
	);
	const pins = events.map((event) => published.get(event.event_id));
	if (pins.some((pin) => !pin)) return null;
	return events.some((event, index) => {
		const pin = pins[index];
		return !!pin && !samePin(pin, event);
	});
}

function driftOf(
	view: AppView | undefined,
	row: AppServiceRow | null,
	mode: AppMode | null,
	events: readonly PlacementEvent[] | null,
): ServiceDrift {
	if (row?.behind != null) return { kind: "count", behind: row.behind };
	// An offline copy's version is the copy itself: event pins can't tell whether its data or flows changed.
	if (!view || !events?.length || mode !== "online") return { kind: "unknown" };
	const behind = pinsBehind(view, events);
	return behind === null ? { kind: "unknown" } : { kind: "pins", behind };
}

export const isBehind = (drift: ServiceDrift) =>
	drift.kind === "count"
		? drift.behind > 0
		: drift.kind === "pins" && drift.behind;

/**
 * What the app's metadata says about a service: the app's name and mode, the
 * events it serves by name, and how far it is behind the versions published now.
 */
export function useServiceApp(
	service: ServiceView | undefined,
	fallbackAppId?: string,
): ServiceApp {
	const { t } = useTranslation("devices");
	const appId = service?.projectId ?? fallbackAppId;
	const { view, loading } = useAppView(appId);
	const appName = useAppNames();
	return useMemo(() => {
		const metadata = view ? "loaded" : loading ? "loading" : "unreadable";
		const listed = appId ? appName(appId) : undefined;
		const fromView =
			view && view.app.name !== appId ? view.app.name : undefined;
		const name =
			fromView ??
			listed ??
			t("service.app.unnamed", "App {{id}}", {
				id: (appId ?? "").slice(0, 8),
			});
		const row =
			(service &&
				view?.services.find(
					(entry) =>
						entry.deviceId === service.deviceId &&
						entry.serviceId === service.serviceId,
				)) ||
			null;
		const mode = row?.mode ?? service?.source ?? view?.app.mode ?? null;
		const meta = new Map(
			view
				? [...view.events.rows, ...view.events.ineligible].map((entry) => [
						entry.eventId,
						entry,
					])
				: [],
		);
		const events = service?.events
			? service.events.map((event) => {
					const known = meta.get(event.event_id);
					return {
						id: event.event_id,
						version: triple(event.event_version),
						...(known ? { name: known.name, eventType: known.eventType } : {}),
					};
				})
			: null;
		return {
			appId,
			name,
			mode,
			view,
			metadata,
			row,
			events,
			drift: driftOf(view, row, mode, service?.events ?? null),
			newestLabel: view?.versions[0]?.label ?? null,
		};
	}, [t, appId, appName, view, loading, service]);
}

/** "Runs online · data stays in the cloud" / "Offline copy · data lives only on {device}, since {date}". */
export function useModeText(app: ServiceApp, device: string): string {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (app.mode === "online")
		return t("service.mode.online", "Runs online · data stays in the cloud");
	if (app.mode !== "offline")
		return t("service.mode.unknown", "Unknown until read live");
	const since = app.row?.data.since;
	return since === undefined
		? t(
				"service.mode.offline",
				"Offline copy · data lives only on {{device}}",
				{
					device,
				},
			)
		: t(
				"service.mode.offlineSince",
				"Offline copy · data lives only on {{device}}, since {{date}}",
				{ device, date: time.at(since) },
			);
}

/** An event the app no longer has keeps its place in the list, without its raw id (R3). */
function eventLabel(t: DevicesT, event: ServedEvent) {
	return event.name ?? t("devices:service.serves.removed", "A removed event");
}

/** APP §7.5 version: label + short hash + drift; never the hash alone when the label is known. */
export function AppVersionValue({
	app,
	service,
}: Readonly<{ app: ServiceApp; service: ServiceView }>) {
	const { t } = useTranslation("devices");
	const version = app.row?.version;
	const hash = version?.hash ?? service.appVersion?.hash;
	const { drift } = app;
	const pinsTitle = (app.events ?? [])
		.map((event) => `${eventLabel(t, event)} ${event.version}`)
		.join(", ");
	return (
		<span className="inline-flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
			{version?.label ? (
				<span className="font-mono">{version.label}</span>
			) : hash ? null : (
				<span className="text-muted-foreground">
					{t("service.version.unknown", "Unknown")}
				</span>
			)}
			{hash ? (
				<IdRef
					id={hash}
					copyLabel={t("service.version.copyHash", "Copy app version hash")}
				/>
			) : null}
			{drift.kind === "pins" && drift.behind ? (
				<StatusChip
					tone="info"
					icon={CircleArrowUp}
					data-drift="behind"
					className="h-5 px-1.5 text-[11.5px]"
					title={t(
						"service.version.behindTitle",
						"Serves {{pins}}. Newer versions of its events are published now.",
						{ pins: pinsTitle },
					)}
				>
					{t("service.version.behind", "Behind")}
				</StatusChip>
			) : (
				<DriftChip
					behind={
						drift.kind === "count"
							? drift.behind
							: drift.kind === "pins"
								? 0
								: null
					}
					{...(drift.kind === "pins"
						? {
								title: t(
									"service.version.newestTitle",
									"Serves the event versions published now.",
								),
							}
						: {})}
				/>
			)}
		</span>
	);
}

const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";

function ServesValue({
	app,
	live,
}: Readonly<{
	app: ServiceApp;
	/** Read over the live connection: only an older agent leaves the event list out there. */
	live: boolean;
}>) {
	const { t } = useTranslation("devices");
	if (!app.events)
		return (
			<span className="text-muted-foreground">
				{live
					? t(
							"service.serves.oldAgent",
							"Unknown: this device's agent is too old to list its events",
						)
					: t(
							"service.serves.snapshot",
							"Unknown: the status snapshot has no event list",
						)}
			</span>
		);
	if (!app.events.length)
		return (
			<span className="text-muted-foreground">
				{t("service.serves.none", "No events")}
			</span>
		);
	if (app.metadata !== "loaded")
		return (
			<span className="text-muted-foreground">
				{app.metadata === "loading"
					? t("service.serves.loading", "Loading event names…")
					: t("service.serves.unnamed", {
							count: app.events.length,
							defaultValue_one:
								"{{count, number}} event; its name shows to people who can open the app",
							defaultValue_other:
								"{{count, number}} events; their names show to people who can open the app",
						})}
			</span>
		);
	return (
		<span className="inline-flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5">
			{app.events.map((event, index) => (
				<Fragment key={event.id}>
					<span>
						{eventLabel(t, event)}{" "}
						<span className="font-mono text-muted-foreground">
							{event.version}
						</span>
						{index < (app.events?.length ?? 0) - 1 ? "," : null}
					</span>
				</Fragment>
			))}
		</span>
	);
}

interface RolloutChipLook {
	tone: ChipTone;
	icon: LucideIcon;
	spin?: boolean;
}

const ROLLOUT_CHIP: Record<string, RolloutChipLook> = {
	staged: { tone: "info", icon: Hourglass },
	validating: { tone: "info", icon: LoaderCircle, spin: true },
	activating: { tone: "info", icon: LoaderCircle, spin: true },
	rolling_back: { tone: "warning", icon: LoaderCircle, spin: true },
	healthy: { tone: "good", icon: CircleCheck },
	rolled_back: { tone: "warning", icon: RotateCcw },
	not_applied: { tone: "warning", icon: CircleSlash },
	failed_stopped: { tone: "critical", icon: OctagonX },
};

function RolloutChip({
	rollout,
	service,
}: Readonly<{ rollout: DeploymentRolloutStatus; service: ServiceView }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const phase = rolloutPhase(rollout);
	if (!phase) return null;
	// The convergence chip already says "Stopped after a failed update".
	if (phase === "failed_stopped" && service.conv === "failed_stopped")
		return null;
	const look = ROLLOUT_CHIP[phase];
	const endedAt = rolloutEndedAt(rollout);
	const label = enumLabel(t, "rollout", rollout.state, {
		failureCode: rollout.failure_code ?? "",
	});
	return (
		<StatusChip
			tone={look.tone}
			icon={look.icon}
			spin={look.spin}
			data-rollout-chip={phase}
		>
			{phase === "healthy" && endedAt !== undefined
				? t("service.header.updatedAt", "Updated {{time}}", {
						time: time.clock(endedAt, false),
					})
				: label}
		</StatusChip>
	);
}

function runWord(t: DevicesT, service: ServiceView, version: number | null) {
	const running =
		service.observed === "running" || service.observed === "starting";
	return running
		? t("devices:service.header.runningVersion", "running v{{version}}", {
				version: version ?? 0,
			})
		: t("devices:service.header.appliedVersion", "applied v{{version}}", {
				version: version ?? 0,
			});
}

/** "Settings v12 → running v11" while they differ, else one outline chip. */
export function SettingsVersion({
	service,
	requestedLabel,
}: Readonly<{ service: ServiceView; requestedLabel?: string }>) {
	const { t } = useTranslation("devices");
	const { applied, latest } = service.settings;
	if (applied === latest || applied === null) return null;
	const requested =
		requestedLabel ??
		t("service.header.settings", "Settings v{{version}}", { version: latest });
	const actual = runWord(t, service, applied);
	return (
		<RequestedActual
			versions
			requested={requested}
			actual={actual}
			label={`${requested}, ${actual}`}
			tone="info"
			icon={RefreshCw}
		/>
	);
}

function ServiceChips({
	service,
	rollout,
}: Readonly<{ service: ServiceView; rollout?: DeploymentRolloutStatus }>) {
	const { t } = useTranslation("devices");
	const { ready, requested, max } = service.instances;
	const diverged =
		service.settings.applied !== null &&
		service.settings.applied !== service.settings.latest;
	return (
		<>
			<RequestedActual
				desired={desiredRun(service)}
				observed={observedRun(service)}
				conv={service.conv}
			/>
			<ConvergenceChip conv={service.conv} />
			<StatusChip tone="outline" icon={Boxes}>
				<span className="tabular-nums">
					{t(
						"service.header.instances",
						"{{ready}} of {{requested}} ready · max {{max}}",
						{ ready, requested, max },
					)}
				</span>
			</StatusChip>
			{diverged ? (
				<SettingsVersion service={service} />
			) : (
				<StatusChip tone="outline" icon={SlidersHorizontal}>
					{t("service.header.settings", "Settings v{{version}}", {
						version: service.settings.latest,
					})}
				</StatusChip>
			)}
			{rollout ? <RolloutChip rollout={rollout} service={service} /> : null}
			<FreshnessStamp {...stampOf(service.freshness)} />
		</>
	);
}

function LockedChips({ device }: Readonly<{ device: DeviceViewModel }>) {
	const { t } = useTranslation("devices");
	const noKeys = device.keys.state === "none";
	return (
		<>
			<KeyChip {...keyChipOf(device.keys, device.live)} />
			{device.presence.kind === "revoked" ? null : (
				<PresenceChip
					kind={device.presence.kind}
					since={device.presence.since}
				/>
			)}
			<FreshnessStamp
				source="snap"
				age={noKeys ? "noaccess" : "locked"}
				text={
					noKeys
						? t("service.header.noKeysHere", "no keys here")
						: t("service.header.locked", "locked")
				}
			/>
		</>
	);
}

const LOCKED_KEYS = new Set([
	"locked",
	"unlocking",
	"held_elsewhere",
	"blocked",
]);

/** While the keys are locked the facts exist but can't be read: say so instead of leaving the line out. */
function lockedFacts(t: DevicesT, device: DeviceViewModel): ObjectFact[] {
	if (!LOCKED_KEYS.has(device.keys.state)) return [];
	const unknown = (
		<span className="text-muted-foreground">
			{t("devices:service.facts.untilUnlocked", "Unknown until unlocked")}
		</span>
	);
	return [
		{
			id: "mode",
			label: t("devices:service.facts.mode", "How it runs"),
			value: unknown,
		},
		{
			id: "serves",
			label: t("devices:service.facts.serves", "Serves"),
			value: unknown,
		},
	];
}

/** A service ID is a name: shown whole (the shared ID chip cuts every ID to eight characters). */
function ServiceIdFact({ serviceId }: Readonly<{ serviceId: string }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	return (
		<span className="inline-flex max-w-full min-w-0 items-center gap-0.5">
			<span className="mr-1 whitespace-nowrap text-muted-foreground">
				{t("service.facts.serviceId", "Service ID")}
			</span>
			<span
				title={serviceId}
				className="min-w-0 truncate rounded-sm border border-hairline bg-surface-sunken px-1.5 py-px font-mono text-xs text-ink-2"
			>
				{serviceId}
			</span>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={copied ? Check : Copy}
				aria-label={
					copied
						? t("service.facts.copied", "Copied")
						: t("service.facts.copyServiceId", "Copy service ID")
				}
				onClick={() => {
					void copy(serviceId);
				}}
				className="size-5.5 text-muted-foreground"
			/>
		</span>
	);
}

const chipLink =
	"rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring [&>span]:hover:border-border-strong";

/** SPEC §5.3 header: the service's id with its app and device, the state chips and the facts line. */
export function ServiceHeader({
	scope,
	deviceId,
	serviceId,
	tab,
	device,
	service,
	app,
	rollout,
}: Readonly<{
	scope: DevicesScope;
	deviceId: string;
	serviceId: string;
	tab: ServiceTab;
	device: DeviceViewModel;
	service: ServiceView | undefined;
	app: ServiceApp;
	rollout?: DeploymentRolloutStatus;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { href, navigate } = useDevicesRoute();
	const overlay = useOverlay();
	const name = deviceName(device.row);
	const modeText = useModeText(app, name);
	const deviceRoute: DevicesRoute = {
		screen: "device",
		deviceId,
		tab: "services",
	};
	const appRoute: DevicesRoute = {
		screen: "app-devices",
		by: "device",
		focusDeviceId: deviceId,
	};

	const crumbTo = (route: DevicesRoute) => ({
		href: href(route),
		onNavigate: () => navigate(route),
	});
	const crumbs: Crumb[] =
		scope.kind === "account"
			? [
					{
						label: t("service.crumbs.devices", "Devices"),
						...crumbTo({ screen: "fleet", view: "devices" }),
					},
					{
						label: <span className="font-mono">{name}</span>,
						...crumbTo(deviceRoute),
					},
					{ label: <span className="font-mono">{serviceId}</span> },
				]
			: [];

	const AppIcon = app.mode ? MODE_ICON[app.mode] : LayoutGrid;
	const nameChips = (
		<span className="inline-flex flex-wrap items-center gap-1.5 text-ui font-normal tracking-normal">
			{app.appId ? (
				<a
					{...link(appRoute, { scope: { kind: "app", appId: app.appId } })}
					title={t(
						"service.header.appTitle",
						"Where {{app}} runs, in its settings",
						{ app: app.name },
					)}
					className={chipLink}
				>
					<StatusChip tone="outline" icon={AppIcon}>
						{app.name}
					</StatusChip>
				</a>
			) : null}
			<a
				{...link(deviceRoute)}
				title={t("service.header.deviceTitle", "Open {{device}}", {
					device: name,
				})}
				className={chipLink}
			>
				<StatusChip
					tone="outline"
					glyph={
						<PresenceGlyph
							kind={device.presence.kind}
							label={enumLabel(t, "presence", device.presence.kind)}
						/>
					}
				>
					<Trans
						t={t}
						i18nKey="service.header.onDevice"
						defaults="on <1>{{device}}</1>"
						values={{ device: name }}
						components={{ 1: <span className="font-mono" /> }}
					/>
				</StatusChip>
			</a>
		</span>
	);

	const others = Array.isArray(device.services)
		? device.services.filter(
				(row) =>
					row.serviceId !== serviceId &&
					(scope.kind === "account" || row.projectId === scope.appId),
			)
		: [];
	const facts: ObjectFact[] = service
		? [
				{
					id: "mode",
					label: t("service.facts.mode", "How it runs"),
					value: (
						<span
							title={
								app.mode ? appCopy(t).mode(app.mode, app.name).sentence : ""
							}
						>
							{modeText}
						</span>
					),
				},
				{
					id: "version",
					label: t("service.facts.version", "App version"),
					value: (
						<>
							<AppVersionValue app={app} service={service} />
							{isBehind(app.drift) && tab !== "configuration" ? (
								<a
									{...link({
										screen: "service",
										deviceId,
										serviceId,
										tab: "configuration",
									})}
									className={`ml-1 ${LINK}`}
									title={t(
										"service.facts.howToUpdateTitle",
										"Configuration explains what an update sends and keeps, and starts it",
									)}
								>
									{t("service.facts.howToUpdate", "How to update")}
								</a>
							) : null}
						</>
					),
				},
				{
					id: "serves",
					label: t("service.facts.serves", "Serves"),
					value: (
						<ServesValue app={app} live={service.freshness.src === "live"} />
					),
				},
			]
		: lockedFacts(t, device);
	const extra = service ? (
		<>
			<ServiceIdFact serviceId={serviceId} />
			{others.length ? (
				<span className="inline-flex min-w-0 flex-wrap items-center gap-x-1">
					<span className="text-muted-foreground">
						{scope.kind === "app"
							? t("service.facts.alsoApp", "Also {{app}} on {{device}}", {
									app: app.name,
									device: name,
								})
							: t("service.facts.also", "Also on {{device}}", { device: name })}
					</span>
					{others.map((row, index) => (
						<Fragment key={row.serviceId}>
							<a
								{...link({
									screen: "service",
									deviceId,
									serviceId: row.serviceId,
									tab,
								})}
								className={`font-mono ${LINK}`}
							>
								{row.serviceId}
							</a>
							{index < others.length - 1 ? "," : null}
						</Fragment>
					))}
				</span>
			) : null}
		</>
	) : null;

	// Keys are the fix only when they are what is missing: a revoked device isn't read, a cloud approval never had keys.
	const readable = !service && device.presence.kind !== "revoked";
	const ownKeys = device.relationship !== "cloud_approval";
	let actions: ReactNode = null;
	if (readable && ownKeys && device.keys.state === "none")
		actions = (
			<DvButton asChild icon={KeyRound}>
				<a {...link({ screen: "keys", focusDeviceId: deviceId })}>
					{t("service.header.restoreKeys", "Restore keys…")}
				</a>
			</DvButton>
		);
	else if (
		readable &&
		device.presence.kind !== "never" &&
		LOCKED_KEYS.has(device.keys.state)
	)
		actions = (
			<DvButton
				variant="primary"
				icon={LockOpen}
				onClick={() => overlay.openUnlock(deviceId)}
			>
				{t("service.header.unlock", "Unlock…")}
			</DvButton>
		);

	return (
		<ObjectHeader
			crumbs={crumbs}
			glyph={
				service ? (
					<PairedPins
						title
						desired={desiredRun(service)}
						observed={observedRun(service)}
						conv={service.conv}
					/>
				) : null
			}
			name={serviceId}
			nameChips={nameChips}
			chips={
				service ? (
					<ServiceChips service={service} rollout={rollout} />
				) : (
					<LockedChips device={device} />
				)
			}
			facts={facts}
			extra={extra}
			actions={actions}
		/>
	);
}

/** App scope: where else the app runs and which devices can't say (APP §6.2 service row). */
export function ServiceAppContextLine({
	appId,
	deviceId,
	serviceId,
	app,
	deviceNames,
}: Readonly<{
	appId: string;
	deviceId: string;
	serviceId: string;
	app: ServiceApp;
	deviceNames: (deviceId: string) => string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const coverage = useCoverage(appId);
	const others = (app.view?.services ?? []).filter(
		(row) => !(row.deviceId === deviceId && row.serviceId === serviceId),
	);
	const never = new Set(coverage.never);
	const unknown = coverage.unknown.filter(
		(id) => id !== deviceId && !never.has(id),
	);
	return (
		<div
			data-service-context=""
			className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-lg border border-border bg-surface-sunken px-3 py-2 text-ui"
		>
			<Layers aria-hidden className="size-4 shrink-0 text-muted-foreground" />
			<span className="min-w-0 flex-1">
				{others.length ? (
					<>
						{t("service.context.alsoRuns", "{{app}} also runs as", {
							app: app.name,
						})}{" "}
						{others.map((row, index) => (
							<Fragment key={`${row.deviceId}/${row.serviceId}`}>
								<a
									{...link({
										screen: "service",
										deviceId: row.deviceId,
										serviceId: row.serviceId,
										tab: "status",
									})}
									className={`font-mono ${LINK}`}
								>
									{deviceNames(row.deviceId)} › {row.serviceId}
								</a>
								{index < others.length - 1 ? ", " : "."}
							</Fragment>
						))}
					</>
				) : (
					t(
						"service.context.nowhereElse",
						"{{app}} runs nowhere else you can see.",
						{ app: app.name },
					)
				)}
				{unknown.length ? (
					<>
						{" "}
						{t("service.context.unknownOn", "Status unknown on {{names}}.", {
							names: formatNames(t, unknown.map(deviceNames), time.locale, 2),
						})}
					</>
				) : null}
			</span>
			<a
				{...link({
					screen: "app-devices",
					by: "device",
					focusDeviceId: deviceId,
				})}
				className={LINK}
			>
				{t("service.context.whereItRuns", "Where it runs")}
			</a>
		</div>
	);
}
