"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Check, Copy, Stethoscope } from "lucide-react";
import { type MouseEvent, type ReactNode, useEffect } from "react";
import { deviceLabel } from "../../../../lib/device-management/model/device-view";
import { CLOCK_SKEW_FLAG_S } from "../../../../lib/device-management/model/freshness";
import type {
	DeviceAuthRejection,
	DeviceViewModel,
	DevicesRoute,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import { liveErrorCode } from "../../../../lib/device-management/workspace/errors";
import type { LiveState } from "../../../../lib/device-management/workspace/types";
import { enumLabel } from "../copy/enum-labels";
import { errorCopy } from "../copy/error-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Checklist, type ChecklistItem } from "../primitives/checklist";
import { CommandBlock } from "../primitives/command-block";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { ageLabel } from "../primitives/freshness-stamp";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import {
	type DesiredRun,
	type ObservedRun,
	RequestedActual,
} from "../primitives/requested-actual";
import { useCopy } from "../primitives/use-copy";
import { devicesHref } from "../routing/devices-href";
import { useAppNames } from "../shell/attention-popover";
import { plainClick } from "../shell/rail-row";
import { useOverlayStore } from "../workspace/overlay-store";
import { useAttentionState } from "../workspace/use-attention";
import { useDeviceView } from "../workspace/use-fleet";
import { useHubSupport } from "../workspace/use-hub";
import type { OverlaySheetProps } from "./area-overlays";

export interface DiagnoseSheetProps extends OverlaySheetProps {
	deviceId: string;
	/** A crashing service: its last known state leads the sheet, or its logs open when the device is reachable. */
	serviceId?: string;
}

/**
 * BG6: the refusal the hub recorded for this device's check-ins, as a sentence.
 * `skew_seconds` is the device clock minus the hub clock.
 */
export function authRejectionText(
	t: DevicesT,
	rejection: DeviceAuthRejection,
	device: string,
	time: Pick<AreaTime, "at">,
): string {
	const refused = t("devices:overlay.diagnose.rejection.refused", {
		count: rejection.count,
		device,
		since: time.at(rejection.first_at),
		defaultValue_one:
			"The hub refused {{count, number}} check-in from {{device}} since {{since}}.",
		defaultValue_other:
			"The hub refused {{count, number}} check-ins from {{device}} since {{since}}.",
	});
	const skew = rejection.skew_seconds;
	const minutes = Math.max(1, Math.round(Math.abs(skew ?? 0) / 60));
	const why =
		rejection.code === "revoked_credential"
			? t(
					"devices:overlay.diagnose.rejection.credential",
					"The device signs in with a credential the hub no longer accepts.",
				)
			: skew == null
				? t(
						"devices:overlay.diagnose.rejection.clock",
						"Its clock is too far off the hub's.",
					)
				: skew > 0
					? t(
							"devices:overlay.diagnose.rejection.clockAhead",
							"Its clock is about {{minutes, number}} min ahead of the hub's.",
							{ minutes },
						)
					: t(
							"devices:overlay.diagnose.rejection.clockBehind",
							"Its clock is about {{minutes, number}} min behind the hub's.",
							{ minutes },
						);
	return `${refused} ${why}`;
}

const OBSERVED: ReadonlySet<string> = new Set<ObservedRun>([
	"unknown",
	"starting",
	"running",
	"stopping",
	"stopped",
	"backoff",
	"failed",
	"removed",
]);

const desiredOf = (service: ServiceView): DesiredRun =>
	service.desired === "stopped" ? "stopped" : "running";
const observedOf = (service: ServiceView): ObservedRun =>
	OBSERVED.has(service.observed)
		? (service.observed as ObservedRun)
		: "unknown";

const reachable = (view: DeviceViewModel) =>
	view.presence.kind === "online" || view.presence.kind === "late";

const SECTION_TITLE = "text-ui font-semibold tracking-normal";
const Mono = ({ children }: Readonly<{ children: ReactNode }>) => (
	<span className="font-mono">{children}</span>
);

interface SheetFacts {
	t: DevicesT;
	time: AreaTime;
	view: DeviceViewModel;
	name: string;
	/** Seconds the device clock looks off, estimated from its last encrypted status. */
	skewS: number | undefined;
}

function hubCheck({ t, time, view, name }: SheetFacts): ChecklistItem {
	const { row } = view;
	if (row.status === "revoked")
		return {
			id: "hub",
			state: "fail",
			source: "hub",
			label:
				row.revoked_at == null
					? t(
							"devices:overlay.diagnose.hub.revoked",
							"{{device}} is revoked on the hub. It can't check in any more.",
							{ device: name },
						)
					: t(
							"devices:overlay.diagnose.hub.revokedAt",
							"{{device}} was revoked on {{at}}. It can't check in any more.",
							{ device: name, at: time.at(row.revoked_at) },
						),
		};
	if (row.last_seen_at == null)
		return {
			id: "hub",
			state: "pass",
			source: "hub",
			label: t(
				"devices:overlay.diagnose.hub.never",
				"The hub knows {{device}}: registered {{ago}}, no check-in yet.",
				{ device: name, ago: time.ago(row.registered_at) },
			),
		};
	return {
		id: "hub",
		state: "pass",
		source: "hub",
		label: t(
			"devices:overlay.diagnose.hub.seen",
			"The hub knows {{device}}. Last check-in {{at}} ({{ago}}).",
			{
				device: name,
				at: time.at(row.last_seen_at),
				ago: time.ago(row.last_seen_at),
			},
		),
	};
}

/** BG6 when the hub recorded a refusal; on hubs without it, the skew estimated from the last encrypted status. */
function clockCheck(facts: SheetFacts): ChecklistItem | undefined {
	const { t, time, view, name, skewS } = facts;
	const rejection = view.row.auth_rejection;
	if (rejection)
		return {
			id: "refused",
			state: "fail",
			source: "hub",
			label: authRejectionText(t, rejection, name, time),
			note:
				rejection.code === "clock_skew"
					? t(
							"devices:overlay.diagnose.rejection.clockFix",
							"Turn on automatic time sync on the device. Check-ins are signed with the time.",
						)
					: t(
							"devices:overlay.diagnose.rejection.credentialFix",
							"Run recover-enrollment on the device (below), or set it up again.",
						),
		};
	if (skewS === undefined || Math.abs(skewS) <= CLOCK_SKEW_FLAG_S)
		return undefined;
	return {
		id: "clock",
		state: "warn",
		source: "snap",
		label: t(
			"devices:overlay.diagnose.clockEstimated",
			"{{device}}'s clock looks {{minutes, number}} min off, estimated from its last encrypted status.",
			{ device: name, minutes: Math.max(1, Math.round(Math.abs(skewS) / 60)) },
		),
		note: t(
			"devices:overlay.diagnose.clockEstimatedNote",
			"More than 5 minutes off and the hub refuses its check-ins. Turn on automatic time sync on the device.",
		),
	};
}

function statusCheck(
	{ t, time, view }: SheetFacts,
	onUnlock: () => void,
): ChecklistItem {
	const { services } = view;
	const id = "status";
	if (Array.isArray(services)) {
		const base = { id, source: "snap" } as const;
		const newest = services.reduce<ServiceView | undefined>(
			(best, row) =>
				(row.freshness.at ?? 0) > (best?.freshness.at ?? 0) ? row : best,
			services[0],
		);
		const stale =
			newest?.freshness.age === "lastknown" ||
			newest?.freshness.age === "snapshot";
		if (!newest || newest.freshness.at === undefined)
			return {
				...base,
				state: "pass",
				label: t(
					"devices:overlay.diagnose.status.readable",
					"The encrypted status is readable on this computer.",
				),
			};
		return {
			...base,
			state: stale ? "warn" : "pass",
			label: stale
				? t(
						"devices:overlay.diagnose.status.lastKnown",
						"Encrypted status from {{at}}, last known.",
						{ at: time.at(newest.freshness.at) },
					)
				: t(
						"devices:overlay.diagnose.status.current",
						"Encrypted status from {{at}}.",
						{ at: time.at(newest.freshness.at) },
					),
		};
	}
	if (view.presence.kind === "never")
		return {
			id,
			source: "hub",
			state: "warn",
			label: t(
				"devices:overlay.diagnose.status.never",
				"No encrypted status yet: the device sends its first one after its first check-in.",
			),
		};
	/* Why it can't be read is a fact about the keys on this computer. */
	const base = { id, source: "local" } as const;
	if (services.state === "locked")
		return {
			...base,
			state: "warn",
			label: t(
				"devices:overlay.diagnose.status.locked",
				"Locked: unlock to read the encrypted status.",
			),
			fix: (
				<DvButton size="xs" onClick={onUnlock}>
					{t("devices:overlay.diagnose.status.unlock", "Unlock…")}
				</DvButton>
			),
		};
	return {
		...base,
		state: "warn",
		label: t(
			"devices:overlay.diagnose.status.unreadable",
			"The encrypted status can't be read on this computer: {{state}}.",
			{ state: ageLabel(t, services.state) },
		),
	};
}

/** Why the live connection is down, as one sentence; `undefined` while there is no failure to report. */
export function liveFailure(t: DevicesT, live: LiveState): string | undefined {
	if (!("cause" in live)) return undefined;
	const { cause } = live;
	const status = cause.step === "getting_pass" ? cause.status : undefined;
	return errorCopy(
		t,
		liveErrorCode(cause),
		status === undefined ? {} : { status },
	);
}

function liveCheck({ t, view }: SheetFacts): ChecklistItem {
	const id = "live";
	const { live, presence } = view;
	if (live.kind === "live" || live.kind === "renewing")
		return {
			id,
			source: "live",
			state: "pass",
			label:
				live.transport === "webrtc"
					? t(
							"devices:overlay.diagnose.live.direct",
							"A live connection is open (direct).",
						)
					: t(
							"devices:overlay.diagnose.live.relayed",
							"A live connection is open (relayed through the hub).",
						),
		};
	const failure = liveFailure(t, live);
	if (failure) return { id, source: "live", state: "fail", label: failure };
	/* Without a session, whether one can be tried follows from the hub's check-in. */
	const base = { id, source: "hub" } as const;
	if (reachable(view))
		return {
			...base,
			state: "pass",
			label: t(
				"devices:overlay.diagnose.live.possible",
				"A live connection can be tried.",
			),
		};
	return {
		...base,
		state: "fail",
		label:
			presence.kind === "never"
				? t(
						"devices:overlay.diagnose.live.never",
						"A live connection won't work: the device has never checked in. It needs a check-in within the last 2 minutes.",
					)
				: presence.kind === "revoked"
					? t(
							"devices:overlay.diagnose.live.revoked",
							"A live connection won't work: the device is revoked.",
						)
					: t(
							"devices:overlay.diagnose.live.offline",
							"A live connection won't work: the device is offline. It needs a check-in within the last 2 minutes.",
						),
	};
}

function keysCheck({ t, view }: SheetFacts): ChecklistItem {
	const base = { id: "keys", source: "local" } as const;
	const { keys } = view;
	if (keys.state === "none")
		return {
			...base,
			state: "fail",
			label: t(
				"devices:overlay.diagnose.keys.none",
				"No keys for this device on this computer.",
			),
		};
	return {
		...base,
		state: keys.state === "stale" || keys.state === "blocked" ? "warn" : "pass",
		label: t(
			"devices:overlay.diagnose.keys.here",
			"{{kind}} here · {{state}}",
			{
				kind: enumLabel(t, "vaultKind", keys.role),
				state: enumLabel(t, "keyState", keys.state),
			},
		),
	};
}

function deviceOnlyCheck(t: DevicesT): ChecklistItem {
	return {
		id: "device",
		state: "pending",
		source: "device",
		label: t(
			"devices:overlay.diagnose.deviceOnly",
			"The connection state, last contact and each service's last error are only on the device.",
		),
	};
}

function commands(t: DevicesT): { command: string; note: string }[] {
	return [
		{
			command: "flow-like-standalone status",
			note: t(
				"devices:overlay.diagnose.command.status",
				"Shows the connection state, last contact and each service's last error.",
			),
		},
		{
			command: "flow-like-standalone service-status",
			note: t(
				"devices:overlay.diagnose.command.serviceStatus",
				"Shows whether the agent runs as a system service and restarts at boot.",
			),
		},
		{
			command: "flow-like-standalone recover-enrollment",
			note: t(
				"devices:overlay.diagnose.command.recover",
				"Use this if status says the hub refused this device.",
			),
		},
		{
			command: "./flow-like-standalone install-service",
			note: t(
				"devices:overlay.diagnose.command.install",
				"Installs the agent as a system service so it starts at boot.",
			),
		},
	];
}

function causes(t: DevicesT, never: boolean, host: string): string[] {
	if (never)
		return [
			t(
				"devices:overlay.diagnose.cause.neverStarted",
				"The agent was never started, or it stopped after the setup window closed.",
			),
			t(
				"devices:overlay.diagnose.cause.clock",
				"The device clock is more than 5 minutes off. Check-ins are signed with the time and the hub refuses them.",
			),
			t(
				"devices:overlay.diagnose.cause.firewall",
				"Outbound HTTPS to {{host}} is blocked by a firewall or proxy.",
				{ host },
			),
			t(
				"devices:overlay.diagnose.cause.otherMachine",
				"The package was started on a different machine than intended.",
			),
		];
	return [
		t("devices:overlay.diagnose.cause.power", "Power or network was lost."),
		t(
			"devices:overlay.diagnose.cause.noService",
			"The agent isn't installed as a system service, so it didn't come back after a restart.",
		),
		t(
			"devices:overlay.diagnose.cause.clockDrift",
			"The device clock drifted more than 5 minutes. Check-ins are signed with the time and the hub refuses them.",
		),
		t(
			"devices:overlay.diagnose.cause.blocked",
			"Outbound HTTPS to {{host}} is blocked.",
			{ host },
		),
	];
}

const iso = (seconds: number | null | undefined) =>
	seconds == null ? "never" : new Date(seconds * 1000).toISOString();

/** Copy diagnostics: English and machine-oriented on purpose (R3), no secrets. */
function report(
	view: DeviceViewModel,
	service: ServiceView | undefined,
	nowS: number,
): string {
	const { row } = view;
	const rejection = row.auth_rejection;
	return [
		`Flow-Like device diagnostics · ${iso(nowS)}`,
		`Device: ${row.name} (${row.device_id})`,
		`Presence: ${view.presence.kind}`,
		`Registered: ${iso(row.registered_at)}`,
		`Last check-in: ${iso(row.last_seen_at)}`,
		`Agent: ${view.agent?.version ?? "not reported"}`,
		`Keys here: ${view.keys.state}`,
		`Live: ${view.live.kind}`,
		rejection
			? `Hub refused check-ins: ${rejection.code} x${rejection.count} since ${iso(rejection.first_at)}, last ${iso(rejection.last_at)}${rejection.skew_seconds == null ? "" : `, skew ${rejection.skew_seconds}s`}`
			: "",
		service
			? `Service ${service.serviceId}: requested ${service.desired}, actual ${service.observed} (${service.freshness.src} ${service.freshness.age})`
			: "",
	]
		.filter(Boolean)
		.join("\n");
}

/** The crashing service as last reported; the reason is only on the device unless the agent reports it (BG7). */
function LastKnownService({ service }: Readonly<{ service: ServiceView }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const appName = useAppNames()(service.projectId);
	const { instances, freshness } = service;
	const lastError = service.diagnostics?.lastError;
	const parts = [
		appName,
		t(
			"overlay.diagnose.service.instances",
			"{{ready, number}} of {{requested, number}} instances ready",
			{ ready: instances.ready, requested: instances.requested },
		),
		freshness.at === undefined
			? ageLabel(t, freshness.age)
			: t("overlay.diagnose.service.reported", "reported {{ago}}", {
					ago: time.ago(freshness.at),
				}),
	].filter(Boolean);
	return (
		<>
			<h3 className={SECTION_TITLE}>
				<Trans
					t={t}
					i18nKey="overlay.diagnose.service.title"
					defaults="Last known: <1/>"
					components={{ 1: <Mono>{service.serviceId}</Mono> }}
				/>
			</h3>
			<RequestedActual
				desired={desiredOf(service)}
				observed={observedOf(service)}
				conv={service.conv}
				sub={parts.join(" · ")}
				className="self-start"
			/>
			{lastError ? (
				<p className="text-xs text-muted-foreground">
					<Trans
						t={t}
						i18nKey="overlay.diagnose.service.lastError"
						defaults="Last error the device reported: <1/>"
						components={{
							1: <span className="font-mono text-foreground">{lastError}</span>,
						}}
					/>
				</p>
			) : (
				<p className="text-xs text-muted-foreground">
					<Trans
						t={t}
						i18nKey="overlay.diagnose.service.onDevice"
						defaults="The reason for the crash is only on the device. Run <1/> there to see it."
						components={{ 1: <Mono>flow-like-standalone status</Mono> }}
					/>
				</p>
			)}
		</>
	);
}

/**
 * SPEC §7.2 `FL.diagnose`: what each data source says about a device that
 * can't be reached, what the hub knows, and what to run on the device. A
 * crashing service on a reachable, unlocked device opens its logs with errors
 * only instead.
 */
export function DiagnoseSheet({
	deviceId,
	serviceId,
	scope,
	onNavigate,
	onClose,
}: Readonly<DiagnoseSheetProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const view = useDeviceView(deviceId, { watch: false });
	const { host } = useHubSupport();
	const { copied, copy } = useCopy();

	const toLogs =
		serviceId !== undefined &&
		view !== undefined &&
		reachable(view) &&
		view.keys.state === "unlocked";
	// biome-ignore lint/correctness/useExhaustiveDependencies: the callbacks are the host's; only the decision re-runs this
	useEffect(() => {
		if (!toLogs || serviceId === undefined) return;
		onClose();
		onNavigate({
			screen: "service",
			deviceId,
			serviceId,
			tab: "activity",
			stream: "errors",
		});
	}, [toLogs, deviceId, serviceId]);
	if (toLogs) return null;

	const name = view
		? view.row.display_name || view.row.name
		: deviceLabel(input, deviceId);
	const deviceName = <Mono>{name}</Mono>;
	const title = (
		<Trans
			t={t}
			i18nKey="overlay.diagnose.title"
			defaults="Diagnose <1/>"
			components={{ 1: deviceName }}
		/>
	);
	const close = (open: boolean) => {
		if (!open) onClose();
	};

	if (!view)
		return (
			<DvSheet
				open
				onOpenChange={close}
				icon={Stethoscope}
				title={title}
				foot={
					<DvButton onClick={onClose}>
						{t("overlay.diagnose.close", "Close")}
					</DvButton>
				}
			>
				<p className="max-w-[72ch] text-sm">
					{t(
						"overlay.diagnose.notListed",
						"This device isn't in your device list on this hub, so there is nothing to check from here. It may have been set up again, or your access ended.",
					)}
				</p>
			</DvSheet>
		);

	const { row, presence } = view;
	const never = row.last_seen_at == null;
	const service = Array.isArray(view.services)
		? view.services.find((entry) => entry.serviceId === serviceId)
		: undefined;
	const facts: SheetFacts = {
		t,
		time,
		view,
		name,
		skewS: input.clock?.deviceSkewS[deviceId],
	};
	const openUnlock = () => useOverlayStore.getState().openUnlock(deviceId);
	const checks = [
		hubCheck(facts),
		clockCheck(facts),
		statusCheck(facts, openUnlock),
		liveCheck(facts),
		keysCheck(facts),
		deviceOnlyCheck(t),
	].filter((item): item is ChecklistItem => item !== undefined);
	const route: DevicesRoute = { screen: "device", deviceId, tab: "overview" };
	const openDevice = (event: MouseEvent) => {
		if (!plainClick(event)) return;
		event.preventDefault();
		onClose();
		onNavigate(route);
	};
	const certificates = view.certificates?.certificates;
	const expired = certificates?.filter(
		(certificate) => certificate.not_after <= time.nowS,
	).length;

	return (
		<DvSheet
			open
			onOpenChange={close}
			wide
			icon={Stethoscope}
			title={title}
			sub={
				never
					? t(
							"overlay.diagnose.subtitleNever",
							"Registered {{ago}} · never checked in",
							{ ago: time.ago(row.registered_at) },
						)
					: presence.kind === "offline"
						? t(
								"overlay.diagnose.subtitleOffline",
								"Offline since {{at}} · last check-in {{ago}}",
								{
									at: time.at(row.last_seen_at as number),
									ago: time.ago(row.last_seen_at as number),
								},
							)
						: t(
								"overlay.diagnose.subtitle",
								"{{presence}} · last check-in {{ago}}",
								{
									presence: enumLabel(t, "presence", presence.kind),
									ago: time.ago(row.last_seen_at as number),
								},
							)
			}
			foot={
				<>
					{/* First in the row, before the sheet's own spacer; on a phone the other two wrap below, right-aligned. */}
					<DvButton
						icon={copied ? Check : Copy}
						className="order-first"
						onClick={() => void copy(report(view, service, time.nowS))}
					>
						{copied
							? t("overlay.diagnose.copied", "Diagnostics copied")
							: t("overlay.diagnose.copy", "Copy diagnostics")}
					</DvButton>
					<DvButton asChild className="ml-auto">
						<a href={devicesHref(route, scope)} onClick={openDevice}>
							<Trans
								t={t}
								i18nKey="overlay.diagnose.open"
								defaults="Open <1/>"
								components={{ 1: deviceName }}
							/>
						</a>
					</DvButton>
					<DvButton onClick={onClose}>
						{t("overlay.diagnose.close", "Close")}
					</DvButton>
				</>
			}
		>
			{service ? <LastKnownService service={service} /> : null}
			<h3 className={SECTION_TITLE}>
				{t("overlay.diagnose.sourcesTitle", "What each source says")}
			</h3>
			<Checklist
				items={checks}
				label={t("overlay.diagnose.sourcesTitle", "What each source says")}
			/>
			<h3 className={SECTION_TITLE}>
				{t("overlay.diagnose.knownTitle", "What the hub knows")}
			</h3>
			<KeyValueList>
				<KvRow label={t("overlay.diagnose.known.registered", "Registered")}>
					{time.at(row.registered_at)}
				</KvRow>
				<KvRow label={t("overlay.diagnose.known.lastCheckIn", "Last check-in")}>
					{never
						? t("overlay.diagnose.known.never", "Never")
						: `${time.at(row.last_seen_at as number)} · ${time.ago(row.last_seen_at as number)}`}
				</KvRow>
				<KvRow
					label={t("overlay.diagnose.known.agent", "Agent")}
					provenance={
						view.agent?.source.at === undefined
							? undefined
							: t(
									"overlay.diagnose.known.agentRead",
									"last live read {{ago}}",
									{
										ago: time.ago(view.agent.source.at),
									},
								)
					}
				>
					{view.agent ? (
						<Mono>{view.agent.version}</Mono>
					) : (
						t("overlay.diagnose.known.notReported", "Not reported")
					)}
				</KvRow>
				<KvRow label={t("overlay.diagnose.known.certificates", "Certificates")}>
					{certificates === undefined
						? t("overlay.diagnose.known.notReported", "Not reported")
						: expired
							? t(
									"overlay.diagnose.known.certificatesExpired",
									"{{count, number}} · {{expired, number}} expired",
									{ count: certificates.length, expired },
								)
							: t(
									"overlay.diagnose.known.certificatesCount",
									"{{count, number}}",
									{
										count: certificates.length,
									},
								)}
				</KvRow>
			</KeyValueList>
			<h3 className={SECTION_TITLE}>
				{t("overlay.diagnose.commandsTitle", "Run these on the device")}
			</h3>
			<p className="text-xs text-muted-foreground">
				{t(
					"overlay.diagnose.commandsHint",
					"These show what only the device knows: its connection state, last contact and each service's last error.",
				)}
			</p>
			<div className="flex flex-col gap-2">
				{commands(t).map((entry) => (
					<CommandBlock
						key={entry.command}
						command={entry.command}
						note={entry.note}
					/>
				))}
			</div>
			{reachable(view) || row.status === "revoked" ? null : (
				<>
					<h3 className={SECTION_TITLE}>
						{t("overlay.diagnose.causesTitle", "Common causes")}
					</h3>
					<ul className="flex list-disc flex-col gap-1 pl-4.5 text-ui">
						{causes(t, never, host).map((cause) => (
							<li key={cause}>{cause}</li>
						))}
					</ul>
				</>
			)}
		</DvSheet>
	);
}
